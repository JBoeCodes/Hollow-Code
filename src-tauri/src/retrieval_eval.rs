//! Real-vault known-item retrieval eval (overnight 10-16 OF01a): how often
//! the host retrieval broker finds a note when asked for it by its title.
//! OB12's gate (BM25 candidates must lift lexical recall@8 to ≥ 35%), and the
//! file OF01b extends with the semantic half ([`Mode`]).
//!
//! The judged corpus in `vault_search_eval` is synthetic; this runs against
//! a real vault named by `HOLLOW_RETRIEVAL_EVAL_VAULT`, in an `#[ignore]`
//! test, so nothing private is committed and no text leaves the machine:
//!
//! ```text
//! T=$(mktemp -d /tmp/hollow-retrieval-eval.XXXXXX); trap 'rm -rf "$T"' EXIT
//! HOLLOW_RETRIEVAL_EVAL_VAULT=<vault copy> TMPDIR="$T" \
//!   cargo test --lib retrieval_eval -- --ignored --nocapture
//! ```
//!
//! (`build-and-ship.md` has the full command.) The run installs a quiet
//! process-global panic hook around its real-request work ([`QuietPanics`]),
//! so run the `#[ignore]` test alone, as above: another test panicking at
//! the same moment would print nothing.
//!
//! **Queries** ([`known_items`]): every `.md` note under [`FOLDERS`]
//! (never a `restricted` folder at any depth, which is skipped unread; never
//! a `_`-prefixed index file), titled the way the vault lists it
//! (`vault_index::classify`: frontmatter `title`, `subject`,
//! `conversation`, then the file stem), with the title embedded in a fixed
//! filler sentence ([`query_for`]). A title with fewer than
//! [`MIN_TITLE_TERMS`] retrieval terms (`directory_connections::
//! retrieval_terms`: words over two letters, stopwords out) is skipped, as is
//! a title whose term set another note in those folders shares (no single
//! right answer). The run keeps at most [`MAX_ITEMS`], evenly spaced over
//! every eligible note in folder then path order, so a folder counts in
//! proportion to its size. No randomness: the same vault gives the same
//! queries.
//!
//! **Retrieval** ([`Mode::Lexical`]): `retrieval_broker::retrieve_lexical`,
//! the broker's public entry point without the semantic half, which runs the
//! same `retrieve_tuned` chat retrieval (`retrieve_by`) does, with the same
//! request (`Scope::all()`, every kind). The real-vault run engages the FTS
//! prefilter first, as the app does (`vault_search_index::engage`); under
//! `cfg(test)` its database root is a per-process temp folder
//! (`vault_search_index::index_root`), so the live SearchIndex is never
//! opened, and the run deletes that folder (it holds note bodies) when it
//! ends ([`SearchIndexRoot`]). The lexical pass never reads the embed index;
//! the semantic half below reads a snapshot of it. Differences from chat: no
//! connected-project files (the test build's manifest root is a temp folder
//! too, so they never compete for the pool) and no Jev rerank. Nothing is
//! written into the vault's notes, but the directory manifest's legacy
//! adoption may remove a `.hollow-directories.json` at the vault root: run it
//! on a copy (`rsync -a`, which keeps the mtimes the ranking reads).
//!
//! **Metrics**: a query's rank is its target's position among the distinct
//! files of the broker's ranked pool (≤ `POOL` = 24 passages), `None` when
//! absent. recall@8 and @24; MRR over the whole pool (0 when absent); nDCG@8
//! with one relevant note (`1 / log2(rank + 1)`, ideal 1); `evidence_recall`,
//! the target cited in the 8-passage evidence budget
//! (`retrieval_broker::evidence`, without Jev's rerank); the broker's
//! wall-clock ms (p50, p95, mean, max; an unoptimised test build, so slower
//! than the app); and how often the vault scan stopped at a budget
//! (`scan_stopped`). Per folder too. Written as JSON to
//! `HOLLOW_RETRIEVAL_EVAL_OUT`, else `$TMPDIR/retrieval-eval-<unix>-<pid>.json`;
//! aggregates only, never a path, title or passage. The report names the
//! commit its numbers come from (`HOLLOW_RETRIEVAL_EVAL_COMMIT`, else
//! `git rev-parse HEAD` of this checkout at run time). **Determinism**: the
//! lexical and blended passes (known-item and replay) each run twice over
//! the same queries and vectors, and `deterministic` (with `determinism`
//! per pass) records whether the rank vectors agreed, ms aside. A mismatch
//! is reported, not a panic, and names no query.
//!
//! **Semantic half** (OF01b, [`VectorIndex`]): when the bge model is found
//! (`HOLLOW_RETRIEVAL_EVAL_MODEL_DIR`, `embed_model::default_model_dir`, then
//! the installed app's; else `semantic: {skipped: "no model"}`), the live
//! EmbedIndex DB (`HOLLOW_RETRIEVAL_EVAL_EMBED_DB`, else derived from
//! `HOLLOW_RETRIEVAL_EVAL_LIVE_VAULT` the way `vault_embed_index` names it)
//! is snapshotted into a fresh temp folder by `replay_probes::snapshot_db`
//! (`VACUUM INTO` from an `immutable=1&mode=ro` connection: the live file is
//! never locked or written, and `EmbedIndex::open`, whose `ensure_schema`
//! drops an index of another version, is never called on it or the copy).
//! The vault is never re-embedded: the `vectors`, `chunks` and `meta` tables
//! are read raw from the snapshot ([`load_vectors`], schema v2 or v3,
//! [`detect_schema`]), every row whose path has a `restricted` component is
//! dropped unread and counted, and the snapshot is deleted
//! ([`Snapshot`]'s `Drop`, also on a panic) before anything is embedded. A
//! snapshot whose `PRAGMA quick_check` isn't `ok` (a torn copy) is not read:
//! the half reports `{skipped: "snapshot quick_check failed"}`
//! ([`through_snapshot`]). Only
//! the queries are embedded (`embed_model::OwnEmbedder`, as the broker
//! embeds a request: its first `SEMANTIC_QUERY_CHARS` characters). Reported:
//! semantic-only ranks (distinct eligible notes by best chunk, no floor) and
//! [`Mode::Blended`], the broker's own `retrieve_with` given those hits, so
//! its `SEMANTIC_MIN` floor, RRF weights and diversity apply unchanged.
//!
//! **Real-query replay** (OF01b): each Jev-labelled run manifest in
//! `HOLLOW_RETRIEVAL_EVAL_MANIFESTS` (default
//! `~/Library/Caches/overnight-20261017/manifests-base`) gives its kept
//! paths ([`jev_run`], as `scripts/agent-eval/retrieval-labels.mjs` reads
//! them); its request is `turns.asked` for the same run id, read from a
//! snapshot of the Sessions DB (`HOLLOW_RETRIEVAL_EVAL_SESSIONS_DB`, else
//! derived from `HOLLOW_RETRIEVAL_EVAL_LIVE_VAULT`) deleted before the broker
//! runs, held in memory only. Only runs whose `retrieval.queryBasis` is
//! `request` (or absent, in manifests predating the field) are replayed:
//! chat queried the others with a wake's subject or a follow-up plus its
//! prior topic, not `asked`, so they are counted by basis and skipped
//! ([`replays_basis`]). Kept paths under `restricted`, outside the vault,
//! no longer eligible for retrieval (`retrieval_broker::vault_eligible`) or
//! missing from the vault copy (drift) are counted and skipped
//! ([`replay_targets`]). recall@8/@24 and MRR of the kept paths, lexical and
//! blended. Each replayed run goes through the broker inside
//! `catch_unwind` with [`QuietPanics`] installed, so a panic can't print the
//! request; panics are counted (`panicked`). A label file
//! (`HOLLOW_RETRIEVAL_EVAL_LABELS`, else
//! `$TMPDIR/retrieval-labels-<unix>-<pid>.json`, [`label_file`]) keeps run
//! id, sha256 of each path, kept and the recorded scores: no path, no text.
//! The hashes are unsalted, so anyone with a listing of the vault can match
//! them back to paths: treat the label file as private as the vault's file
//! names.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};

use crate::retrieval_broker::{
    SemanticHit, LEXICAL_WEIGHT, POOL, RRF_K, SEMANTIC_MIN, SEMANTIC_NOTES, SEMANTIC_WEIGHT,
};

/// The folders whose notes become known-item queries.
pub(crate) const FOLDERS: &[&str] = &["library", "tasks", "learnings"];
/// Items per run at most, evenly spaced over every eligible note (folder by
/// folder, sorted paths), so each folder counts in proportion to its size.
pub(crate) const MAX_ITEMS: usize = 300;
/// A title needs this many retrieval terms to identify its note.
pub(crate) const MIN_TITLE_TERMS: usize = 1;
/// The filler around a title: `<before><title><after>`.
pub(crate) const FILLER_BEFORE: &str =
    "before tomorrow's meeting could you pull together everything we have on ";
pub(crate) const FILLER_AFTER: &str =
    " including notes, tasks, decisions and open questions so I can review it";
/// The cut-offs reported.
const RECALL_KS: [usize; 2] = [8, 24];
const NDCG_K: usize = 8;

/// Which half of retrieval answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The broker without its semantic half.
    Lexical,
    /// The broker with its semantic half answered from the eval's own read
    /// of the embed index ([`VectorIndex::app_hits`]), fused by the broker's
    /// `retrieve_with` (its floor, RRF weights and diversity, not a copy).
    Blended,
}

/// Why a [`Mode::Blended`] query had no semantic answer.
const NOT_EMBEDDED: &str = "retrieval eval: the request couldn't be embedded";

impl Mode {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Mode::Lexical => "lexical",
            Mode::Blended => "blended",
        }
    }

    /// The broker's ranked pool for `query`, as chat asks for it. `hits` is
    /// the semantic half's answer for [`Mode::Blended`] (`None`: the query
    /// couldn't be embedded); [`Mode::Lexical`] ignores it.
    fn retrieve(
        self,
        vault: &Path,
        query: &str,
        hits: Option<&[SemanticHit]>,
    ) -> crate::retrieval_broker::Ranked {
        let scope = crate::retrieval_broker::Scope::all();
        let req = crate::retrieval_broker::Request {
            vault,
            query,
            scope: &scope,
            kinds: None,
        };
        match self {
            Mode::Lexical => {
                crate::retrieval_broker::retrieve_lexical(&req, "retrieval eval: lexical half only")
            }
            Mode::Blended => {
                let answer: crate::retrieval_broker::SemanticAnswer = match hits {
                    Some(hits) => Ok(hits.to_vec()),
                    None => Err(NOT_EMBEDDED.to_string()),
                };
                crate::retrieval_broker::retrieve_with(
                    &req,
                    &|texts: &[String], notes: &[crate::types::Note]| {
                        crate::vault_search_index::ranked_candidates(vault, texts, notes)
                    },
                    crate::vault_search_index::prefilter_kind(vault),
                    &|_: &str| answer.clone(),
                )
            }
        }
    }
}

/// One known-item query: the note it must find.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub folder: &'static str,
    /// Vault-relative, as `vault::walk_notes` gives it.
    pub rel: String,
    pub query: String,
}

/// Why notes in [`FOLDERS`] didn't become queries (counts only).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Skipped {
    pub restricted: usize,
    pub unreadable: usize,
    pub short_title: usize,
    pub duplicate_title: usize,
    /// Over [`MAX_ITEMS`].
    pub capped: usize,
}

/// The query for a note titled `title`.
pub(crate) fn query_for(title: &str) -> String {
    format!("{FILLER_BEFORE}{}{FILLER_AFTER}", title.trim())
}

/// A title's retrieval terms, as the broker would read them.
pub(crate) fn title_terms(title: &str) -> Vec<String> {
    crate::directory_connections::retrieval_terms(title)
        .into_iter()
        .map(|t| t.text)
        .collect()
}

/// `n` items evenly spaced over `v` (all of it when it fits), in order.
fn spaced<T>(v: Vec<T>, n: usize) -> Vec<T> {
    let len = v.len();
    if len <= n {
        return v;
    }
    let keep: std::collections::HashSet<usize> = (0..n).map(|i| i * len / n).collect();
    v.into_iter()
        .enumerate()
        .filter(|(i, _)| keep.contains(i))
        .map(|(_, x)| x)
        .collect()
}

/// The known-item queries for `vault` (see the module doc), sorted by path
/// within each folder, folders in [`FOLDERS`] order.
pub(crate) fn known_items(vault: &Path) -> (Vec<Item>, Skipped) {
    let mut skipped = Skipped::default();
    // (folder, rel, title, term key)
    let mut found: Vec<(&'static str, String, String, String)> = Vec::new();
    for note in crate::vault::walk_notes(vault) {
        let folder = crate::vault_index::top_folder(&note.path);
        let Some(folder) = FOLDERS.iter().copied().find(|f| *f == folder) else {
            continue;
        };
        if !note.path.ends_with(".md") || note.name.starts_with('_') {
            continue;
        }
        if Path::new(&note.path)
            .components()
            .any(|c| c.as_os_str() == "restricted")
        {
            skipped.restricted += 1;
            continue;
        }
        let Ok(content) = std::fs::read_to_string(vault.join(&note.path)) else {
            skipped.unreadable += 1;
            continue;
        };
        let (fm, _) = crate::frontmatter::parse_map(&content);
        let title = crate::vault_index::classify(&note.path, &fm).title;
        let mut terms = title_terms(&title);
        if terms.len() < MIN_TITLE_TERMS {
            skipped.short_title += 1;
            continue;
        }
        terms.sort();
        found.push((folder, note.path, title, terms.join(" ")));
    }
    let mut shared: HashMap<String, usize> = HashMap::new();
    for (_, _, _, key) in &found {
        *shared.entry(key.clone()).or_default() += 1;
    }
    let mut items = Vec::new();
    for &folder in FOLDERS {
        let mut mine: Vec<Item> = Vec::new();
        for (f, rel, title, key) in &found {
            if *f != folder {
                continue;
            }
            if shared[key] > 1 {
                skipped.duplicate_title += 1;
                continue;
            }
            mine.push(Item {
                folder,
                rel: rel.clone(),
                query: query_for(title),
            });
        }
        mine.sort_by(|a, b| a.rel.cmp(&b.rel));
        items.extend(mine);
    }
    if items.len() > MAX_ITEMS {
        skipped.capped += items.len() - MAX_ITEMS;
        items = spaced(items, MAX_ITEMS);
    }
    (items, skipped)
}

/// The vault-relative path of a broker passage, or its own (absolute) path
/// for a connected project file.
fn rel_of(vault: &Path, path: &str) -> String {
    Path::new(path)
        .strip_prefix(vault)
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

/// The 1-based position of `target` among the distinct files of `passages`,
/// in order; `None` when it isn't there.
pub(crate) fn note_rank(
    vault: &Path,
    passages: &[crate::directory_connections::Passage],
    target: &str,
) -> Option<usize> {
    let mut seen = std::collections::HashSet::new();
    for p in passages {
        let rel = rel_of(vault, &p.path);
        if rel == target {
            return Some(seen.len() + 1);
        }
        seen.insert(rel);
    }
    None
}

/// The share of queries whose target ranked within `k`.
pub(crate) fn recall_at(ranks: &[Option<usize>], k: usize) -> f64 {
    if ranks.is_empty() {
        return 0.0;
    }
    ranks.iter().filter(|r| r.is_some_and(|r| r <= k)).count() as f64 / ranks.len() as f64
}

/// Mean reciprocal rank (a miss counts 0).
pub(crate) fn mrr(ranks: &[Option<usize>]) -> f64 {
    if ranks.is_empty() {
        return 0.0;
    }
    ranks
        .iter()
        .map(|r| r.map_or(0.0, |r| 1.0 / r as f64))
        .sum::<f64>()
        / ranks.len() as f64
}

/// Mean nDCG@k with one relevant note per query (ideal DCG 1).
pub(crate) fn ndcg_at(ranks: &[Option<usize>], k: usize) -> f64 {
    if ranks.is_empty() {
        return 0.0;
    }
    ranks
        .iter()
        .map(|r| match r {
            Some(r) if *r <= k => 1.0 / ((*r as f64) + 1.0).log2(),
            _ => 0.0,
        })
        .sum::<f64>()
        / ranks.len() as f64
}

/// The nearest-rank percentile `p` (0–100) of `ms`; 0 when empty.
fn percentile(ms: &[u64], p: f64) -> u64 {
    if ms.is_empty() {
        return 0;
    }
    let mut v = ms.to_vec();
    v.sort_unstable();
    let i = ((p / 100.0) * v.len() as f64).ceil() as usize;
    v[i.clamp(1, v.len()) - 1]
}

/// One query's result.
#[derive(Clone, Debug)]
pub(crate) struct Outcome {
    pub folder: &'static str,
    pub rank: Option<usize>,
    /// The target is cited in the 8 passages chat sends.
    pub in_evidence: bool,
    pub ms: u64,
    pub prefilter: String,
    /// Why the broker's vault scan stopped early (a budget), if it did.
    pub stopped: Option<String>,
    /// The broker's semantic state (`off`, `blended`, `unavailable`).
    pub semantic: String,
}

/// The semantic half for [`Mode::Blended`]: the index and each item's query
/// vector (`None` where it couldn't be embedded), index-aligned with the
/// items.
pub(crate) type SemanticInput<'a> = (&'a VectorIndex, &'a [Option<Vec<f32>>]);

/// The semantic answer for item `i` (see [`SemanticInput`]): `None` without
/// a semantic half or a vector.
fn hits_for(semantic: Option<SemanticInput>, i: usize) -> Option<Vec<SemanticHit>> {
    let (index, qvecs) = semantic?;
    let qvec = qvecs.get(i)?.as_ref()?;
    Some(index.app_hits(&index.scored(qvec), SEMANTIC_NOTES))
}

/// Every item through the broker in `mode`. The semantic lookup runs
/// before the broker's clock starts (the app runs it beside the scan).
pub(crate) fn run(
    vault: &Path,
    mode: Mode,
    items: &[Item],
    semantic: Option<SemanticInput>,
) -> Vec<Outcome> {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let hits = hits_for(semantic, i);
            let started = Instant::now();
            let ranked = mode.retrieve(vault, &item.query, hits.as_deref());
            let ms = crate::evidence::ms_since(started);
            let rank = note_rank(vault, &ranked.passages, &item.rel);
            let in_evidence = crate::retrieval_broker::evidence(ranked.passages)
                .iter()
                .any(|p| rel_of(vault, &p.path) == item.rel);
            Outcome {
                folder: item.folder,
                rank,
                in_evidence,
                ms,
                prefilter: ranked.stats.prefilter,
                stopped: ranked.stats.stopped,
                semantic: ranked.stats.semantic,
            }
        })
        .collect()
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// The metrics of `outcomes` as JSON.
fn metrics(outcomes: &[Outcome]) -> serde_json::Value {
    let ranks: Vec<Option<usize>> = outcomes.iter().map(|o| o.rank).collect();
    let ms: Vec<u64> = outcomes.iter().map(|o| o.ms).collect();
    let evidence = if outcomes.is_empty() {
        0.0
    } else {
        outcomes.iter().filter(|o| o.in_evidence).count() as f64 / outcomes.len() as f64
    };
    let mean = if ms.is_empty() {
        0.0
    } else {
        ms.iter().sum::<u64>() as f64 / ms.len() as f64
    };
    let mut m = serde_json::json!({
        "n": outcomes.len(),
        "found_in_pool": ranks.iter().filter(|r| r.is_some()).count(),
        "mrr": round4(mrr(&ranks)),
        "ndcg_at_8": round4(ndcg_at(&ranks, NDCG_K)),
        "evidence_recall": round4(evidence),
        "broker_ms": {
            "p50": percentile(&ms, 50.0),
            "p95": percentile(&ms, 95.0),
            "mean": round4(mean),
            "max": ms.iter().copied().max().unwrap_or(0),
        },
    });
    for k in RECALL_KS {
        m[format!("recall_at_{k}")] = serde_json::json!(round4(recall_at(&ranks, k)));
    }
    m
}

/// One mode's broker outcomes: totals, per folder, the prefilter, budget
/// stops and the broker's semantic state, each counted.
fn summary(outcomes: &[Outcome]) -> serde_json::Value {
    let mut per_folder = serde_json::Map::new();
    for folder in FOLDERS {
        let mine: Vec<Outcome> = outcomes
            .iter()
            .filter(|o| o.folder == *folder)
            .cloned()
            .collect();
        per_folder.insert((*folder).into(), metrics(&mine));
    }
    let mut prefilters: BTreeMap<String, usize> = BTreeMap::new();
    // A scan stopped by a budget can't rank notes it never read: OB12 needs
    // to tell a ranking gain from a budget artifact.
    let mut stopped: BTreeMap<String, usize> = BTreeMap::new();
    let mut semantic: BTreeMap<String, usize> = BTreeMap::new();
    for o in outcomes {
        *prefilters.entry(o.prefilter.clone()).or_default() += 1;
        let why = o.stopped.clone().unwrap_or_else(|| "none".into());
        *stopped.entry(why).or_default() += 1;
        *semantic.entry(o.semantic.clone()).or_default() += 1;
    }
    serde_json::json!({
        "prefilter": prefilters,
        "scan_stopped": stopped,
        "semantic_state": semantic,
        "total": metrics(outcomes),
        "per_folder": per_folder,
    })
}

// ── Miss diagnosis (RC2) ───────────────────────────────────────────────────

/// The stage at which a known note failed to reach the agent (RC2). Each
/// miss reports exactly one, the earliest that explains it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MissStage {
    /// Recall can't see the note at all; the reason is the rule that hides it.
    NotIndexed(&'static str),
    /// Indexed and scanned, but not within the broker's ranked pool.
    Outranked,
    /// In the pool, then cut by a budget: the scan stopped before reading it
    /// (the reason is the broker's), or it ranked past the passages chat sends.
    CutByBudget(String),
    /// Delivered to the agent, whose answer did not use it (only when the
    /// caller knows: the eval has no answer text).
    DeliveredUnused,
}

impl MissStage {
    /// The key a report counts it under.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            MissStage::NotIndexed(_) => "not_indexed",
            MissStage::Outranked => "outranked",
            MissStage::CutByBudget(_) => "cut_by_budget",
            MissStage::DeliveredUnused => "delivered_unused",
        }
    }

    /// One short sentence naming the cause.
    pub(crate) fn why(&self) -> String {
        match self {
            MissStage::NotIndexed(rule) => format!("not indexed: {rule}"),
            MissStage::Outranked => "indexed but outranked: not within the ranked pool".into(),
            MissStage::CutByBudget(reason) => format!("ranked but cut by a budget: {reason}"),
            MissStage::DeliveredUnused => "delivered but unused by the answer".into(),
        }
    }
}

/// The rule that keeps vault-relative `rel` out of the search index, or
/// `None` when it is indexable: the rules the vault walk and the broker apply
/// (a `restricted` folder, a hidden component or a skipped top-level folder,
/// not a note file (`.md`, `.html`, `.htm`), over the size limit, missing or
/// unreadable).
pub(crate) fn not_indexed_reason(vault: &Path, rel: &str) -> Option<&'static str> {
    if restricted(rel) {
        return Some("it is under a restricted folder");
    }
    if crate::vault::walk_skips(rel) {
        return Some("it is in a hidden or skipped folder");
    }
    let lower = rel.to_lowercase();
    if ![".md", ".html", ".htm"]
        .iter()
        .any(|ext| lower.ends_with(ext))
    {
        return Some("it is not a note file");
    }
    match std::fs::metadata(vault.join(rel)) {
        Err(_) => Some("it is missing or unreadable"),
        Ok(meta) if !meta.is_file() => Some("it is not a file"),
        Ok(meta) if meta.len() > crate::vault_search::RETRIEVAL_MAX_NOTE_BYTES => {
            Some("it is over the size limit")
        }
        Ok(_) => None,
    }
}

/// Where the known note `rel` was lost, or `None` when it reached the agent.
/// `used`: whether the answer used it, when the caller has the answer.
pub(crate) fn diagnose_miss(
    vault: &Path,
    rel: &str,
    outcome: &Outcome,
    used: Option<bool>,
) -> Option<MissStage> {
    if let Some(rule) = not_indexed_reason(vault, rel) {
        return Some(MissStage::NotIndexed(rule));
    }
    match (outcome.rank, outcome.in_evidence) {
        (None, _) => Some(match &outcome.stopped {
            Some(reason) => MissStage::CutByBudget(format!("the scan stopped: {reason}")),
            None => MissStage::Outranked,
        }),
        (Some(rank), false) => Some(MissStage::CutByBudget(format!(
            "it ranked {rank}, past the passages chat sends"
        ))),
        (Some(_), true) if used == Some(false) => Some(MissStage::DeliveredUnused),
        (Some(_), true) => None,
    }
}

/// A run's misses by stage, with the one-line summary the doctor shows.
/// `items` and `outcomes` are index-aligned (`run`'s order). Never names a
/// note: only counts.
pub(crate) fn miss_report(vault: &Path, items: &[Item], outcomes: &[Outcome]) -> serde_json::Value {
    let mut stages: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (item, outcome) in items.iter().zip(outcomes) {
        if let Some(stage) = diagnose_miss(vault, &item.rel, outcome, None) {
            *stages.entry(stage.name()).or_default() += 1;
        }
    }
    let total: usize = stages.values().sum();
    serde_json::json!({
        "total": total,
        "of": outcomes.len(),
        "stages": stages,
        "doctor_line": miss_line(total, outcomes.len(), &stages),
    })
}

/// "Recall missed 3 of 120 notes: 1 not indexed, 2 outranked." One line, for
/// the capability doctor and the eval report.
pub(crate) fn miss_line(total: usize, of: usize, stages: &BTreeMap<&'static str, usize>) -> String {
    if total == 0 {
        return format!("Recall found all {of} known notes.");
    }
    let parts: Vec<String> = stages
        .iter()
        .map(|(name, n)| format!("{n} {}", name.replace('_', " ")))
        .collect();
    format!("Recall missed {total} of {of} notes: {}.", parts.join(", "))
}

/// The report for one run: totals, per folder, and how the queries were made.
/// OF01b's `semantic`, `blended` and `replay` blocks are added beside it.
pub(crate) fn report(
    mode: Mode,
    outcomes: &[Outcome],
    skipped: &Skipped,
    commit: Option<String>,
    unix: u64,
) -> serde_json::Value {
    let s = summary(outcomes);
    serde_json::json!({
        "schema": "hollow.retrieval-eval/2",
        "mode": mode.name(),
        "commit": commit,
        "generated_unix": unix,
        "query_template": format!("{FILLER_BEFORE}<title>{FILLER_AFTER}"),
        "folders": FOLDERS,
        "caps": {
            "total": MAX_ITEMS,
            "min_title_terms": MIN_TITLE_TERMS,
        },
        "skipped": {
            "restricted": skipped.restricted,
            "unreadable": skipped.unreadable,
            "short_title": skipped.short_title,
            "duplicate_title": skipped.duplicate_title,
            "capped": skipped.capped,
        },
        "rank_unit": "distinct files in the broker's ranked pool (at most 24 passages)",
        "prefilter": s["prefilter"],
        "scan_stopped": s["scan_stopped"],
        "semantic_state": s["semantic_state"],
        "total": s["total"],
        "per_folder": s["per_folder"],
    })
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The commit the numbers come from: `HOLLOW_RETRIEVAL_EVAL_COMMIT`, else
/// `git rev-parse HEAD` of this checkout (`-dirty` appended when tracked
/// files differ), else `None`.
fn commit() -> Option<String> {
    if let Some(c) = env_nonempty("HOLLOW_RETRIEVAL_EVAL_COMMIT") {
        return Some(c);
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let head = git(&["rev-parse", "HEAD"]).filter(|h| !h.is_empty())?;
    // `--no-optional-locks`: `status` would otherwise refresh the index and
    // take `index.lock`, racing a lane's own git commands in this checkout.
    let dirty = git(&[
        "--no-optional-locks",
        "status",
        "--porcelain",
        "--untracked-files=no",
    ])
    .is_some_and(|s| !s.is_empty());
    Some(if dirty { format!("{head}-dirty") } else { head })
}

// ── Snapshots ─────────────────────────────────────────────────────────────

/// A snapshot of a live SQLite database in a fresh temp folder
/// (`hollow-test-retrieval-eval-<pid>-<random>`, mode 0700), made by
/// `replay_probes::snapshot_db` (`VACUUM INTO` from an `immutable=1&mode=ro`
/// connection: the live file is never locked or written; frames the app
/// hasn't checkpointed aren't in it). Dropping it deletes the folder, also
/// on an error or a panic.
pub(crate) struct Snapshot {
    pub dir: PathBuf,
    pub db: PathBuf,
    /// The snapshot's `PRAGMA quick_check`: `ok`, or SQLite's messages.
    pub quick_check: String,
}

impl Snapshot {
    /// Snapshot `live` as `<label>.db` under `TMPDIR` ([`Snapshot::take_in`]).
    pub(crate) fn take(live: &Path, label: &str) -> Result<Snapshot, String> {
        Snapshot::take_in(&std::env::temp_dir(), live, label)
    }

    /// Snapshot `live` as `<label>.db` in a fresh folder under `tmp`.
    /// Refuses (creating nothing) a `tmp` inside this checkout, a worktree
    /// or `~/Hollow` (`replay_probes::refuse_temp_dir`).
    pub(crate) fn take_in(tmp: &Path, live: &Path, label: &str) -> Result<Snapshot, String> {
        use std::os::unix::fs::PermissionsExt;
        if !live.is_file() {
            return Err(format!("the {label} database is not a file"));
        }
        crate::replay_probes::refuse_temp_dir(tmp)?;
        let dir = tmp.join(format!(
            "hollow-test-retrieval-eval-{}-{:08x}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&dir).map_err(|e| format!("snapshot folder: {e}"))?;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        // From here the guard owns the folder: an error below deletes it.
        let mut snap = Snapshot {
            db: dir.join(format!("{label}.db")),
            dir,
            quick_check: String::new(),
        };
        snap.quick_check = crate::replay_probes::snapshot_db(live, &snap.db)?;
        Ok(snap)
    }

    /// A read-only connection to the snapshot.
    pub(crate) fn conn(&self) -> Result<Connection, String> {
        Connection::open_with_flags(
            &self.db,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("snapshot open: {e}"))
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        // Always the fresh folder `take` made, never the live database.
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The size of the `<db>-wal` file beside a live database (0 when there is
/// none). Not the bytes a snapshot leaves out: SQLite doesn't shrink the WAL
/// file after a checkpoint, so it can be large with nothing pending. Only
/// its being 0 proves the snapshot complete; quitting Hollow checkpoints and
/// removes it.
fn wal_file_bytes(db: &Path) -> u64 {
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    std::fs::metadata(PathBuf::from(wal))
        .map(|m| m.len())
        .unwrap_or(0)
}

/// What [`through_snapshot`] read, and the snapshot's facts.
pub(crate) struct Snapshotted<T> {
    /// `read`'s answer; `Err("snapshot quick_check failed")` without
    /// calling it when the copy is torn.
    pub value: Result<T, String>,
    /// [`wal_file_bytes`] of the live database, before the snapshot.
    pub live_wal_file_bytes: u64,
    pub quick_check: String,
    /// The snapshot's folder, already deleted. For tests: never reported.
    pub dir: PathBuf,
}

impl<T> Snapshotted<T> {
    /// The facts for the report: sizes and SQLite's verdict, no path.
    pub(crate) fn facts(&self) -> Value {
        json!({
            "live_wal_file_bytes": self.live_wal_file_bytes,
            "snapshot_quick_check": self.quick_check,
            "snapshot_deleted": !self.dir.exists(),
        })
    }
}

/// Why a half was skipped over a torn snapshot.
pub(crate) const QUICK_CHECK_FAILED: &str = "snapshot quick_check failed";

/// `read` over a read-only connection to a [`Snapshot`] of `live`, which is
/// deleted before this returns (asserted), whatever `read` answered. A
/// snapshot whose `quick_check` isn't `ok` isn't read. `Err` only when no
/// snapshot could be taken.
pub(crate) fn through_snapshot<T>(
    live: &Path,
    label: &str,
    read: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<Snapshotted<T>, String> {
    let live_wal_file_bytes = wal_file_bytes(live);
    let snap = Snapshot::take(live, label)?;
    let dir = snap.dir.clone();
    let quick_check = snap.quick_check.clone();
    let value = if quick_check == "ok" {
        snap.conn().and_then(|conn| read(&conn))
    } else {
        Err(QUICK_CHECK_FAILED.to_string())
    };
    drop(snap);
    assert!(!dir.exists(), "the {label} snapshot was not deleted");
    Ok(Snapshotted {
        value,
        live_wal_file_bytes,
        quick_check,
        dir,
    })
}

// ── Leak guards ───────────────────────────────────────────────────────────

/// The panic hook a run had before [`QuietPanics`].
type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

/// A panic hook that prints nothing, for the span of real-request work: a
/// panic inside the broker or the tokenizer (a char-boundary slice, say)
/// would otherwise print a message that can quote the request on stderr.
/// Pair it with [`quietly`] per item. Dropping it restores the previous
/// hook, also on an early return. `set_hook` is process-global, so the
/// real-vault test, which is `#[ignore]`, must run alone (the module doc's
/// command does): another test's panic message would be lost meanwhile.
pub(crate) struct QuietPanics {
    previous: Option<PanicHook>,
}

impl QuietPanics {
    pub(crate) fn install() -> QuietPanics {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        QuietPanics {
            previous: Some(previous),
        }
    }
}

impl Drop for QuietPanics {
    fn drop(&mut self) {
        // `set_hook` panics on a panicking thread, which would abort; the
        // quiet hook stays then (the panic was already silenced).
        if std::thread::panicking() {
            return;
        }
        if let Some(previous) = self.previous.take() {
            std::panic::set_hook(previous);
        }
    }
}

/// `f`, with a panic caught and its payload dropped unprinted: `None` when it
/// panicked. Prints nothing only under [`QuietPanics`].
fn quietly<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// The test build's FTS index root (`vault_search_index::index_root`, a
/// per-process temp folder) that `engage` fills with note bodies: deleted
/// when this drops, at the end of the run or on a panic. Only a folder named
/// `hollow-test-searchindex-*` is ever removed.
pub(crate) struct SearchIndexRoot(Option<PathBuf>);

impl SearchIndexRoot {
    pub(crate) fn new() -> SearchIndexRoot {
        SearchIndexRoot(crate::vault_search_index::index_root())
    }
}

impl Drop for SearchIndexRoot {
    fn drop(&mut self) {
        let Some(root) = self.0.take() else {
            return;
        };
        let ours = root
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("hollow-test-searchindex-"));
        if ours {
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}

/// A live per-vault database: `env` when set, else
/// `~/Library/Application Support/Hollow/<folder>/<vault stem>.db` for
/// `HOLLOW_RETRIEVAL_EVAL_LIVE_VAULT` (`session_store::vault_file_stem`,
/// which `vault_embed_index` names its databases by too).
fn live_db(env: &str, folder: &str) -> Option<PathBuf> {
    if let Some(path) = env_nonempty(env) {
        return Some(PathBuf::from(path));
    }
    let live = env_nonempty("HOLLOW_RETRIEVAL_EVAL_LIVE_VAULT")?;
    let stem = crate::session_store::vault_file_stem(Path::new(&live));
    Some(
        dirs::data_dir()?
            .join("Hollow")
            .join(folder)
            .join(format!("{stem}.db")),
    )
}

/// Whether vault-relative `rel` has a `restricted` component.
fn restricted(rel: &str) -> bool {
    crate::replay_probes::has_restricted(Path::new(rel))
}

// ── Semantic half (OF01b) ─────────────────────────────────────────────────

/// The embed index schema versions [`load_vectors`] reads: v2 (`vectors`
/// beside the chunk store) and v3 (HF03's `chunks.in_frontmatter`).
pub(crate) const EMBED_SCHEMAS: [i64; 2] = [2, 3];

/// The column names of `table` (empty when it doesn't exist).
fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

/// The embed index's schema version and how it was told: `user_version`
/// when set (it must agree with the columns), else `columns` (v3 when
/// `chunks` has `in_frontmatter`, v2 otherwise). An error names why it
/// can't be read: no `vectors` or `chunks` table, or a version outside
/// [`EMBED_SCHEMAS`].
pub(crate) fn detect_schema(conn: &Connection) -> Result<(i64, &'static str), String> {
    let version: i64 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .map_err(|e| format!("user_version: {e}"))?;
    let chunks = columns(conn, "chunks");
    let vectors = columns(conn, "vectors");
    let meta = columns(conn, "meta");
    if chunks.is_empty() || vectors.is_empty() || meta.is_empty() {
        return Err(format!(
            "no chunks, vectors or meta table (user_version {version})"
        ));
    }
    let inferred = if chunks.iter().any(|c| c == "in_frontmatter") {
        3
    } else {
        2
    };
    match version {
        0 => Ok((inferred, "columns")),
        v if EMBED_SCHEMAS.contains(&v) && v == inferred => Ok((v, "user_version")),
        v if EMBED_SCHEMAS.contains(&v) => {
            Err(format!("schema v{v} with v{inferred} columns: not read"))
        }
        v => Err(format!("unsupported embed index schema v{v}")),
    }
}

/// One embedded chunk: what `EmbedIndex::note_hits` needs of it, and its
/// vector. Never its text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VectorRow {
    pub path: String,
    pub ord: usize,
    pub kind: String,
    pub mtime: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub in_frontmatter: bool,
    pub vec: Vec<f32>,
}

/// The embed index's vectors, read raw from a snapshot ([`load_vectors`]),
/// and what was left out.
#[derive(Clone, Debug, Default)]
pub(crate) struct VectorIndex {
    pub schema: i64,
    /// `user_version` or `columns` ([`detect_schema`]).
    pub schema_basis: &'static str,
    /// Sorted by path, then chunk order.
    pub rows: Vec<VectorRow>,
    /// Rows in `vectors`, before anything was left out.
    pub vectors_total: usize,
    /// Rows (and distinct notes) under a `restricted` folder: dropped by
    /// path before their vector is read.
    pub restricted_rows: usize,
    pub restricted_notes: usize,
    /// `dim = 0`: an embed that failed.
    pub tombstones: usize,
    /// A blob whose length isn't `dim * 4`.
    pub malformed: usize,
}

/// Read every embedded chunk of the embed index behind `conn` (a snapshot,
/// opened read-only): `vectors` joined to its `chunks` row (offsets, and
/// `in_frontmatter` on v3; never `text`) and its note's `meta` (kind,
/// mtime). Rows under `restricted` are counted and dropped by path alone.
pub(crate) fn load_vectors(conn: &Connection) -> Result<VectorIndex, String> {
    let (schema, schema_basis) = detect_schema(conn)?;
    let vectors_total: i64 = conn
        .query_row("SELECT COUNT(*) FROM vectors;", [], |r| r.get(0))
        .map_err(|e| format!("count vectors: {e}"))?;
    let frontmatter = if schema >= 3 { "c.in_frontmatter" } else { "0" };
    let sql = format!(
        "SELECT v.path, v.ord, v.dim, c.byte_start, c.byte_end, {frontmatter}, \
         COALESCE(m.kind, ''), COALESCE(m.mtime, '') \
         FROM vectors v JOIN chunks c ON c.path = v.path AND c.ord = v.ord \
         LEFT JOIN meta m ON m.path = v.path \
         ORDER BY v.path, v.ord;"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("read vectors: {e}"))?;
    let mut blob = conn
        .prepare("SELECT vec FROM vectors WHERE path = ?1 AND ord = ?2;")
        .map_err(|e| format!("read vectors: {e}"))?;
    let mut rows = stmt.query([]).map_err(|e| format!("read vectors: {e}"))?;
    let mut index = VectorIndex {
        schema,
        schema_basis,
        vectors_total: vectors_total.max(0) as usize,
        ..Default::default()
    };
    let mut restricted_notes: BTreeSet<String> = BTreeSet::new();
    while let Some(r) = rows.next().map_err(|e| format!("read vectors: {e}"))? {
        let path: String = r.get(0).map_err(|e| format!("read vectors: {e}"))?;
        if restricted(&path) {
            index.restricted_rows += 1;
            restricted_notes.insert(path);
            continue;
        }
        let get_usize = |i: usize| -> Result<usize, String> {
            r.get::<_, i64>(i)
                .map(|v| v.max(0) as usize)
                .map_err(|e| format!("read vectors: {e}"))
        };
        let ord = get_usize(1)?;
        let dim = get_usize(2)?;
        if dim == 0 {
            index.tombstones += 1;
            continue;
        }
        let bytes: Vec<u8> = blob
            .query_row(rusqlite::params![path, ord as i64], |b| b.get(0))
            .map_err(|e| format!("read vectors: {e}"))?;
        if bytes.len() != dim * 4 {
            index.malformed += 1;
            continue;
        }
        let vec = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        index.rows.push(VectorRow {
            path,
            ord,
            byte_start: get_usize(3)?,
            byte_end: get_usize(4)?,
            in_frontmatter: r.get(5).map_err(|e| format!("read vectors: {e}"))?,
            kind: r.get(6).map_err(|e| format!("read vectors: {e}"))?,
            mtime: r.get(7).map_err(|e| format!("read vectors: {e}"))?,
            vec,
        });
    }
    index.restricted_notes = restricted_notes.len();
    Ok(index)
}

impl VectorIndex {
    /// Every row of `qvec`'s dimension as `(cosine, row)`, best first, as
    /// `EmbedIndex::query` scores them (dot product of L2-normalized
    /// vectors, summed in the same order); ties by row (path, then chunk
    /// order), so the order is total.
    pub(crate) fn scored(&self, qvec: &[f32]) -> Vec<(f32, usize)> {
        let mut out: Vec<(f32, usize)> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.vec.len() == qvec.len())
            .map(|(i, r)| {
                let mut dot = 0.0f32;
                for (q, v) in qvec.iter().zip(&r.vec) {
                    dot += q * v;
                }
                (dot, i)
            })
            .collect();
        out.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        out
    }

    /// What `EmbedIndex::note_hits` answers the broker for `limit` notes:
    /// the best `limit * 4` chunks of `scored`, the first per note, at most
    /// `limit` notes. No floor or eligibility here: the broker applies both.
    pub(crate) fn app_hits(&self, scored: &[(f32, usize)], limit: usize) -> Vec<SemanticHit> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut out = Vec::new();
        for &(score, i) in scored.iter().take(limit.saturating_mul(4).max(limit)) {
            let r = &self.rows[i];
            if !seen.insert(r.path.as_str()) {
                continue;
            }
            out.push(SemanticHit {
                path: r.path.clone(),
                kind: r.kind.clone(),
                mtime: r.mtime.clone(),
                byte_start: r.byte_start,
                byte_end: r.byte_end,
                in_frontmatter: r.in_frontmatter,
                score,
            });
            if out.len() >= limit {
                break;
            }
        }
        out
    }

    /// The semantic-only ranking: distinct notes passing `keep`, by their
    /// best chunk in `scored`, at most `limit`, with that chunk's cosine. No
    /// floor and no overfetch cap.
    pub(crate) fn ranking(
        &self,
        scored: &[(f32, usize)],
        limit: usize,
        keep: &dyn Fn(&str) -> bool,
    ) -> Vec<(String, f32)> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut out = Vec::new();
        for &(score, i) in scored {
            if out.len() >= limit {
                break;
            }
            let path = self.rows[i].path.as_str();
            if !keep(path) || !seen.insert(path) {
                continue;
            }
            out.push((path.to_string(), score));
        }
        out
    }
}

/// One known-item query's semantic-only answer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SemanticOutcome {
    pub folder: &'static str,
    /// Among distinct eligible notes ([`VectorIndex::ranking`], ≤ `POOL`).
    pub rank: Option<usize>,
    /// The target is in what the broker would get ([`VectorIndex::app_hits`])
    /// at or above `SEMANTIC_MIN`.
    pub reaches_blend: bool,
    pub embedded: bool,
}

/// Every item's semantic-only answer from `index` (`qvecs` index-aligned
/// with `items`; `None`: not embedded, a miss).
pub(crate) fn run_semantic(
    index: &VectorIndex,
    items: &[Item],
    qvecs: &[Option<Vec<f32>>],
) -> Vec<SemanticOutcome> {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let Some(qvec) = qvecs.get(i).and_then(|q| q.as_ref()) else {
                return SemanticOutcome {
                    folder: item.folder,
                    rank: None,
                    reaches_blend: false,
                    embedded: false,
                };
            };
            let scored = index.scored(qvec);
            let ranking = index.ranking(&scored, POOL, &crate::retrieval_broker::vault_eligible);
            let rank = ranking
                .iter()
                .position(|(p, _)| *p == item.rel)
                .map(|i| i + 1);
            let reaches_blend = index
                .app_hits(&scored, SEMANTIC_NOTES)
                .iter()
                .any(|h| h.path == item.rel && h.score >= SEMANTIC_MIN);
            SemanticOutcome {
                folder: item.folder,
                rank,
                reaches_blend,
                embedded: true,
            }
        })
        .collect()
}

/// The semantic-only metrics of `outcomes`.
fn semantic_metrics(outcomes: &[SemanticOutcome]) -> Value {
    let ranks: Vec<Option<usize>> = outcomes.iter().map(|o| o.rank).collect();
    let share = |n: usize| {
        if outcomes.is_empty() {
            0.0
        } else {
            n as f64 / outcomes.len() as f64
        }
    };
    let mut m = json!({
        "n": outcomes.len(),
        "embedded": outcomes.iter().filter(|o| o.embedded).count(),
        "found_in_top": ranks.iter().filter(|r| r.is_some()).count(),
        "mrr": round4(mrr(&ranks)),
        "ndcg_at_8": round4(ndcg_at(&ranks, NDCG_K)),
        "reaches_blend": round4(share(outcomes.iter().filter(|o| o.reaches_blend).count())),
    });
    for k in RECALL_KS {
        m[format!("recall_at_{k}")] = json!(round4(recall_at(&ranks, k)));
    }
    m
}

/// [`semantic_metrics`] in total and per folder.
fn semantic_summary(outcomes: &[SemanticOutcome]) -> Value {
    let mut per_folder = serde_json::Map::new();
    for folder in FOLDERS {
        let mine: Vec<SemanticOutcome> = outcomes
            .iter()
            .filter(|o| o.folder == *folder)
            .cloned()
            .collect();
        per_folder.insert((*folder).into(), semantic_metrics(&mine));
    }
    json!({
        "rank_unit": "distinct eligible notes by best chunk cosine (at most 24), no floor",
        "total": semantic_metrics(outcomes),
        "per_folder": per_folder,
    })
}

/// The broker's fusion settings, as the report records them.
fn fusion() -> Value {
    json!({
        "semantic_min": SEMANTIC_MIN,
        "semantic_notes": SEMANTIC_NOTES,
        "rrf_k": RRF_K,
        "lexical_weight": LEXICAL_WEIGHT,
        "semantic_weight": SEMANTIC_WEIGHT,
    })
}

/// Query embeddings: index-aligned vectors (`None` where embedding failed
/// or panicked), each one's ms, and how many panicked.
pub(crate) struct Embedded {
    pub vecs: Vec<Option<Vec<f32>>>,
    pub ms: Vec<u64>,
    pub panicked: usize,
}

/// `texts` embedded as the broker embeds a request (the first
/// `SEMANTIC_QUERY_CHARS` characters; `embed` adds the bge query prefix).
/// Each embed runs in [`quietly`]: a panic is a `None` and counted.
fn embed_all(texts: &[&str], embed: &dyn Fn(&str) -> Result<Vec<f32>, String>) -> Embedded {
    let mut out = Embedded {
        vecs: Vec::with_capacity(texts.len()),
        ms: Vec::with_capacity(texts.len()),
        panicked: 0,
    };
    for text in texts {
        let started = Instant::now();
        let vec = quietly(|| {
            let cut: String = text
                .chars()
                .take(crate::retrieval_broker::SEMANTIC_QUERY_CHARS)
                .collect();
            embed(&cut).ok()
        });
        out.ms.push(crate::evidence::ms_since(started));
        out.panicked += usize::from(vec.is_none());
        out.vecs.push(vec.flatten());
    }
    out
}

/// The bge model folder: `HOLLOW_RETRIEVAL_EVAL_MODEL_DIR`, this build's
/// (`embed_model::default_model_dir`), then the installed app's; the first
/// with `model.safetensors`.
fn model_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = env_nonempty("HOLLOW_RETRIEVAL_EVAL_MODEL_DIR") {
        candidates.push(PathBuf::from(dir));
    }
    candidates.push(crate::embed_model::default_model_dir());
    candidates.push(PathBuf::from(
        "/Applications/Hollow.app/Contents/Resources/models/bge-small-en-v1.5",
    ));
    candidates
        .into_iter()
        .find(|d| d.join("model.safetensors").is_file())
}

/// The live embed index read through a snapshot deleted before this returns
/// ([`through_snapshot`]): the index and the snapshot's facts, or the
/// `{skipped, snapshot}` block saying why not (a torn snapshot is skipped,
/// not read).
fn read_embed_index(live: &Path) -> Result<(VectorIndex, Value), Value> {
    let read =
        through_snapshot(live, "embed", load_vectors).map_err(|why| json!({ "skipped": why }))?;
    let facts = read.facts();
    match read.value {
        Ok(index) => Ok((index, facts)),
        Err(why) => Err(json!({ "skipped": why, "snapshot": facts })),
    }
}

/// The index's facts for the report: counts only.
fn index_facts(index: &VectorIndex) -> Value {
    let notes: HashSet<&str> = index.rows.iter().map(|r| r.path.as_str()).collect();
    json!({
        "schema_version": index.schema,
        "schema_basis": index.schema_basis,
        "vectors_total": index.vectors_total,
        "rows_used": index.rows.len(),
        "notes": notes.len(),
        "skipped": {
            "restricted_rows": index.restricted_rows,
            "restricted_notes": index.restricted_notes,
            "tombstones": index.tombstones,
            "malformed": index.malformed,
        },
    })
}

// ── Real-query replay (OF01b) ─────────────────────────────────────────────

/// One Jev-decided retrieval candidate of a run manifest.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct JevCandidate {
    /// Vault-relative for a vault note; absolute for a project file.
    pub path: String,
    pub rank: u64,
    pub local_score: Option<f64>,
    pub semantic_score: Option<f64>,
    pub jev_score: Option<f64>,
    pub kept: bool,
}

/// A run whose retrieval Jev labelled: its id and Jev-decided candidates,
/// by recorded rank.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct JevRun {
    pub run_id: String,
    pub candidates: Vec<JevCandidate>,
    /// `retrieval.queryBasis`: what chat's query was read from (`request`,
    /// `hostSubject`, `followup+prior`, ...); `None` in manifests predating
    /// the field.
    pub query_basis: Option<String>,
    /// Jev-basis candidates without a string path, a rank or a boolean
    /// `kept`: left out, counted.
    pub malformed: usize,
}

/// Whether a run with `basis` can be replayed from its `turns.asked`: chat
/// queried with the request itself only on basis `request` (or absent, in
/// manifests predating the field); a wake's subject or a follow-up plus its
/// prior topic is not what `asked` holds.
pub(crate) fn replays_basis(basis: Option<&str>) -> bool {
    matches!(basis, None | Some("request"))
}

/// A manifest's Jev-labelled run, as `retrieval-labels.mjs` reads one
/// (`retrieval.engine == "jev"`; candidates with `basis == "jev"`, a rank
/// and a boolean `kept`, the rest counted as `malformed`), or why it has
/// none: `no_candidates`, `not_jev`, `malformed` (Jev candidates, none
/// well-formed), `no_run_id`.
pub(crate) fn jev_run(manifest: &Value) -> Result<JevRun, &'static str> {
    let r = &manifest["retrieval"];
    let Some(all) = r["candidates"].as_array().filter(|c| !c.is_empty()) else {
        return Err("no_candidates");
    };
    if r["engine"].as_str() != Some("jev") {
        return Err("not_jev");
    }
    let Some(run_id) = manifest["runId"].as_str().filter(|s| !s.is_empty()) else {
        return Err("no_run_id");
    };
    let jev: Vec<&Value> = all
        .iter()
        .filter(|c| c["basis"].as_str() == Some("jev"))
        .collect();
    let mut candidates: Vec<JevCandidate> = jev
        .iter()
        .filter_map(|c| {
            Some(JevCandidate {
                path: c["path"].as_str()?.to_string(),
                rank: c["rank"].as_u64()?,
                local_score: c["localScore"].as_f64(),
                semantic_score: c["semanticScore"].as_f64(),
                jev_score: c["jevScore"].as_f64(),
                kept: c["kept"].as_bool()?,
            })
        })
        .collect();
    let malformed = jev.len() - candidates.len();
    if candidates.is_empty() {
        return Err(if malformed > 0 {
            "malformed"
        } else {
            "not_jev"
        });
    }
    candidates.sort_by_key(|c| c.rank);
    Ok(JevRun {
        run_id: run_id.to_string(),
        candidates,
        query_basis: r["queryBasis"].as_str().map(str::to_string),
        malformed,
    })
}

/// Every `.json` manifest in `dir`, by file name: the Jev-labelled runs and
/// the count of each kind of file (`files`, `unreadable`, `labelled` and
/// [`jev_run`]'s reasons), plus `malformed_candidates` over the labelled
/// runs.
pub(crate) fn load_manifests(dir: &Path) -> Result<(Vec<JevRun>, BTreeMap<String, usize>), String> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("manifests folder: {e}"))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    names.sort();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut runs = Vec::new();
    for path in names {
        *counts.entry("files".into()).or_default() += 1;
        let parsed = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let Some(manifest) = parsed else {
            *counts.entry("unreadable".into()).or_default() += 1;
            continue;
        };
        match jev_run(&manifest) {
            Ok(run) => {
                *counts.entry("labelled".into()).or_default() += 1;
                *counts.entry("malformed_candidates".into()).or_default() += run.malformed;
                runs.push(run);
            }
            Err(why) => *counts.entry(why.into()).or_default() += 1,
        }
    }
    Ok((runs, counts))
}

/// Why kept paths didn't become replay targets (counts only).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeptSkips {
    pub restricted: usize,
    /// An absolute path: a connected project's file (the eval has none).
    pub project_file: usize,
    /// A `..`, `.` or root component: never looked up.
    pub invalid: usize,
    /// No longer eligible for retrieval (`retrieval_broker::vault_eligible`:
    /// `me/`, `sessions/`, `workflows/`, a workspace's non-output files):
    /// the broker can't return it, so it would be a miss by construction.
    pub ineligible: usize,
    /// No longer in the vault copy.
    pub drift: usize,
}

/// The distinct kept paths of `run` that are files in `vault` (sorted), each
/// other kept path counted in `skips`: `restricted` first (never looked
/// up), then project files and invalid paths, then paths the broker no
/// longer considers, then drift.
pub(crate) fn replay_targets(vault: &Path, run: &JevRun, skips: &mut KeptSkips) -> Vec<String> {
    let kept: BTreeSet<&str> = run
        .candidates
        .iter()
        .filter(|c| c.kept)
        .map(|c| c.path.as_str())
        .collect();
    let mut targets = Vec::new();
    for path in kept {
        let rel = Path::new(path);
        if restricted(path) {
            skips.restricted += 1;
        } else if rel.is_absolute() {
            skips.project_file += 1;
        } else if !rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            skips.invalid += 1;
        } else if !crate::retrieval_broker::vault_eligible(path) {
            skips.ineligible += 1;
        } else if !vault.join(rel).is_file() {
            skips.drift += 1;
        } else {
            targets.push(path.to_string());
        }
    }
    targets
}

/// The hex sha256 of `text`.
pub(crate) fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// The label file for `runs`: per run its id and per Jev-decided candidate
/// the sha256 of its path, `kept` and the recorded scores. No path and no
/// text; a candidate under `restricted` is left out (and counted). The
/// hashes are unsalted: a listing of the vault maps them back to paths.
pub(crate) fn label_file(runs: &[JevRun]) -> (Value, usize) {
    let mut left_out = 0;
    let runs: Vec<Value> = runs
        .iter()
        .map(|run| {
            let candidates: Vec<Value> = run
                .candidates
                .iter()
                .filter(|c| {
                    let keep = !restricted(&c.path);
                    left_out += usize::from(!keep);
                    keep
                })
                .map(|c| {
                    json!({
                        "pathSha256": sha256_hex(&c.path),
                        "kept": c.kept,
                        "rank": c.rank,
                        "localScore": c.local_score,
                        "semanticScore": c.semantic_score,
                        "jevScore": c.jev_score,
                    })
                })
                .collect();
            json!({ "runId": run.run_id, "candidates": candidates })
        })
        .collect();
    (
        json!({
            "schema": "hollow.retrieval-labels/1",
            "runs": runs,
            "restrictedLeftOut": left_out,
        }),
        left_out,
    )
}

/// The newest non-empty `turns.asked` of each of `run_ids` in the Sessions
/// DB snapshot behind `conn`. In memory only: never written or logged.
fn requests_for(conn: &Connection, run_ids: &[&str]) -> Result<HashMap<String, String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT asked FROM turns WHERE run_id = ?1 AND asked IS NOT NULL \
             AND trim(asked) <> '' ORDER BY id DESC LIMIT 1;",
        )
        .map_err(|e| format!("read turns: {e}"))?;
    let mut out = HashMap::new();
    for id in run_ids {
        let asked: Option<String> = stmt
            .query_row([id], |r| r.get(0))
            .optional()
            .map_err(|e| format!("read turns: {e}"))?;
        if let Some(asked) = asked {
            out.insert((*id).to_string(), asked);
        }
    }
    Ok(out)
}

/// One replayed run: its request (in memory only) and kept target paths.
pub(crate) struct ReplayItem {
    pub request: String,
    pub targets: Vec<String>,
}

/// One replayed run's answer: each target's rank (distinct files in the
/// pool) and the broker's ms.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReplayOutcome {
    pub ranks: Vec<Option<usize>>,
    pub ms: u64,
}

/// Every replay item through the broker in `mode` (as [`run`]), each in
/// [`quietly`]: `None` where the broker panicked. The caller installs
/// [`QuietPanics`] around real requests so the panic prints nothing.
pub(crate) fn run_replay(
    vault: &Path,
    mode: Mode,
    items: &[ReplayItem],
    semantic: Option<SemanticInput>,
) -> Vec<Option<ReplayOutcome>> {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            quietly(|| {
                let hits = hits_for(semantic, i);
                let started = Instant::now();
                let ranked = mode.retrieve(vault, &item.request, hits.as_deref());
                let ms = crate::evidence::ms_since(started);
                let ranks = item
                    .targets
                    .iter()
                    .map(|t| note_rank(vault, &ranked.passages, t))
                    .collect();
                ReplayOutcome { ranks, ms }
            })
        })
        .collect()
}

/// The rank vectors of a pass, ms aside, for the determinism check.
fn replay_ranks(outcomes: &[Option<ReplayOutcome>]) -> Vec<Option<Vec<Option<usize>>>> {
    outcomes
        .iter()
        .map(|o| o.as_ref().map(|o| o.ranks.clone()))
        .collect()
}

/// The rank vector of a known-item pass, ms aside, for the determinism
/// check.
fn ranks_of(outcomes: &[Outcome]) -> Vec<Option<usize>> {
    outcomes.iter().map(|o| o.rank).collect()
}

/// The best `target_mrr` reachable: a run with N kept targets can at best
/// rank them 1..N, scoring `1 + 1/2 + … + 1/N` over its N targets.
fn target_mrr_ceiling(outcomes: &[&ReplayOutcome]) -> f64 {
    let targets: usize = outcomes.iter().map(|o| o.ranks.len()).sum();
    if targets == 0 {
        return 0.0;
    }
    let best: f64 = outcomes
        .iter()
        .map(|o| (1..=o.ranks.len()).map(|k| 1.0 / k as f64).sum::<f64>())
        .sum();
    best / targets as f64
}

/// The replay metrics over the runs that didn't panic (`panicked` counts the
/// rest): per run first (the best-ranked target's reciprocal rank, any
/// target within 8 / 24), then per kept target (recall@8/@24, MRR, and the
/// MRR ceiling several targets per run impose).
pub(crate) fn replay_metrics(outcomes: &[Option<ReplayOutcome>]) -> Value {
    let panicked = outcomes.iter().filter(|o| o.is_none()).count();
    let outcomes: Vec<&ReplayOutcome> = outcomes.iter().flatten().collect();
    let targets: Vec<Option<usize>> = outcomes.iter().flat_map(|o| o.ranks.clone()).collect();
    let best: Vec<Option<usize>> = outcomes
        .iter()
        .map(|o| o.ranks.iter().flatten().min().copied())
        .collect();
    let ms: Vec<u64> = outcomes.iter().map(|o| o.ms).collect();
    json!({
        "runs": outcomes.len(),
        "panicked": panicked,
        "targets": targets.len(),
        "target_recall_at_8": round4(recall_at(&targets, 8)),
        "target_recall_at_24": round4(recall_at(&targets, 24)),
        "target_mrr": round4(mrr(&targets)),
        "target_mrr_ceiling": round4(target_mrr_ceiling(&outcomes)),
        "target_mrr_note": "per kept target; a run with N targets can't reach 1 (see target_mrr_ceiling): lead with run_mrr_first",
        "run_mrr_first": round4(mrr(&best)),
        "run_any_at_8": round4(recall_at(&best, 8)),
        "run_any_at_24": round4(recall_at(&best, 24)),
        "broker_ms": {
            "p50": percentile(&ms, 50.0),
            "p95": percentile(&ms, 95.0),
        },
    })
}

/// The semantic half, ready: the index and a way to embed a query.
struct SemanticReady {
    index: VectorIndex,
    embedder: crate::embed_model::OwnEmbedder,
}

/// The semantic half with the index facts and the snapshot's facts
/// (`live_wal_file_bytes`, `snapshot_quick_check`, `snapshot_deleted`), or
/// the `{skipped}` block saying why not (with `snapshot` and `index` when
/// they were read).
fn semantic_setup() -> Result<(SemanticReady, Value, Value), Value> {
    let Some(dir) = model_dir() else {
        return Err(json!({ "skipped": "no model" }));
    };
    let Some(live) = live_db("HOLLOW_RETRIEVAL_EVAL_EMBED_DB", "EmbedIndex") else {
        return Err(json!({
            "skipped": "no embed index: set HOLLOW_RETRIEVAL_EVAL_EMBED_DB or HOLLOW_RETRIEVAL_EVAL_LIVE_VAULT"
        }));
    };
    let (index, snapshot) = read_embed_index(&live)?;
    let facts = index_facts(&index);
    if index.rows.is_empty() {
        return Err(json!({
            "skipped": "the embed index has no usable vectors",
            "index": facts,
            "snapshot": snapshot,
        }));
    }
    let Ok(embedder) = crate::embed_model::OwnEmbedder::load(&dir) else {
        return Err(json!({
            "skipped": "the model didn't load",
            "index": facts,
            "snapshot": snapshot,
        }));
    };
    Ok((SemanticReady { index, embedder }, facts, snapshot))
}

/// The replay block: the label file written, the runs replayed lexical and
/// (with the semantic half) blended, or `{skipped}`.
fn replay(vault: &Path, semantic: Option<&SemanticReady>, unix: u64) -> Value {
    let dir = env_nonempty("HOLLOW_RETRIEVAL_EVAL_MANIFESTS")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|c| c.join("overnight-20261017/manifests-base")));
    let Some(dir) = dir.filter(|d| d.is_dir()) else {
        return json!({ "skipped": "no manifests folder: set HOLLOW_RETRIEVAL_EVAL_MANIFESTS" });
    };
    let (runs, manifests) = match load_manifests(&dir) {
        Ok(found) => found,
        Err(why) => return json!({ "skipped": why }),
    };
    let (labels, restricted_left_out) = label_file(&runs);
    let label_out = env_nonempty("HOLLOW_RETRIEVAL_EVAL_LABELS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "retrieval-labels-{unix}-{}.json",
                std::process::id()
            ))
        });
    std::fs::write(&label_out, serde_json::to_string_pretty(&labels).unwrap())
        .expect("write the retrieval label file");
    eprintln!("retrieval_eval: label file -> {}", label_out.display());
    let mut block = json!({
        "manifests": manifests,
        "label_candidates_restricted_left_out": restricted_left_out,
    });
    let Some(sessions) = live_db("HOLLOW_RETRIEVAL_EVAL_SESSIONS_DB", "Sessions") else {
        block["skipped"] =
            "no Sessions DB: set HOLLOW_RETRIEVAL_EVAL_SESSIONS_DB or HOLLOW_RETRIEVAL_EVAL_LIVE_VAULT"
                .into();
        return block;
    };
    // Only runs chat queried with their own request are replayed: the others'
    // query (a wake's subject, a follow-up plus its prior) isn't `asked`.
    let mut skipped_basis: BTreeMap<String, usize> = BTreeMap::new();
    let runs: Vec<&JevRun> = runs
        .iter()
        .filter(|run| {
            let replayed = replays_basis(run.query_basis.as_deref());
            if !replayed {
                let basis = run.query_basis.clone().unwrap_or_default();
                *skipped_basis.entry(basis).or_default() += 1;
            }
            replayed
        })
        .collect();
    // The requests, in memory; the snapshot is deleted before the broker runs
    // and isn't read when torn.
    let ids: Vec<&str> = runs.iter().map(|r| r.run_id.as_str()).collect();
    let read = match through_snapshot(&sessions, "sessions", |conn| requests_for(conn, &ids)) {
        Ok(read) => read,
        Err(why) => {
            block["skipped"] = why.into();
            return block;
        }
    };
    // Turns still only in the live WAL aren't in the snapshot: they show as
    // `runs.request_missing`.
    block["sessions_snapshot"] = read.facts();
    let requests = match read.value {
        Ok(found) => found,
        Err(why) => {
            block["skipped"] = why.into();
            return block;
        }
    };
    let mut skips = KeptSkips::default();
    let (mut request_missing, mut no_targets, mut kept_total) = (0usize, 0usize, 0usize);
    let mut items: Vec<ReplayItem> = Vec::new();
    for run in runs.iter().copied() {
        let kept: BTreeSet<&str> = run
            .candidates
            .iter()
            .filter(|c| c.kept)
            .map(|c| c.path.as_str())
            .collect();
        kept_total += kept.len();
        let Some(request) = requests.get(&run.run_id) else {
            request_missing += 1;
            continue;
        };
        let targets = replay_targets(vault, run, &mut skips);
        if targets.is_empty() {
            no_targets += 1;
            continue;
        }
        items.push(ReplayItem {
            request: request.clone(),
            targets,
        });
    }
    drop(requests);
    block["runs"] = json!({
        "labelled": runs.len() + skipped_basis.values().sum::<usize>(),
        "skipped_basis": skipped_basis,
        "request_missing": request_missing,
        "no_targets": no_targets,
        "replayed": items.len(),
    });
    block["kept"] = json!({
        "total": kept_total,
        "restricted": skips.restricted,
        "project_file": skips.project_file,
        "invalid": skips.invalid,
        "ineligible": skips.ineligible,
        "drift": skips.drift,
        "targets": items.iter().map(|i| i.targets.len()).sum::<usize>(),
    });
    block["note"] = "kept.total counts the runs of a replayed basis; for runs without a \
                     request it is their only count"
        .into();
    // Real requests from here on: a panic is caught per run and prints
    // nothing; the hook comes back when `_quiet` drops.
    let _quiet = QuietPanics::install();
    let lexical = run_replay(vault, Mode::Lexical, &items, None);
    let lexical_again = run_replay(vault, Mode::Lexical, &items, None);
    let mut deterministic = json!({
        "lexical": replay_ranks(&lexical) == replay_ranks(&lexical_again),
        "blended": null,
    });
    block["lexical"] = replay_metrics(&lexical);
    block["blended"] = match semantic {
        Some(ready) => {
            let texts: Vec<&str> = items.iter().map(|i| i.request.as_str()).collect();
            let embedded = embed_all(&texts, &|t: &str| ready.embedder.query(t));
            let input = Some((&ready.index, embedded.vecs.as_slice()));
            let blended = run_replay(vault, Mode::Blended, &items, input);
            let blended_again = run_replay(vault, Mode::Blended, &items, input);
            let agreed = replay_ranks(&blended) == replay_ranks(&blended_again);
            deterministic["blended"] = agreed.into();
            let mut m = replay_metrics(&blended);
            m["embedded"] = embedded.vecs.iter().filter(|q| q.is_some()).count().into();
            m["embed_panicked"] = embedded.panicked.into();
            m["embed_ms_p50"] = percentile(&embedded.ms, 50.0).into();
            m
        }
        None => json!({ "skipped": "no semantic half" }),
    };
    block["deterministic"] = deterministic;
    block
}

/// The real-vault run. Skips (passes) when `HOLLOW_RETRIEVAL_EVAL_VAULT` is
/// unset.
#[test]
#[ignore = "reads a real vault: HOLLOW_RETRIEVAL_EVAL_VAULT=<vault>"]
fn real_vault_known_item_retrieval() {
    let Some(vault) = env_nonempty("HOLLOW_RETRIEVAL_EVAL_VAULT") else {
        eprintln!("retrieval_eval: HOLLOW_RETRIEVAL_EVAL_VAULT is unset; skipped");
        return;
    };
    let vault = PathBuf::from(vault);
    assert!(
        vault.is_dir(),
        "HOLLOW_RETRIEVAL_EVAL_VAULT is not a folder"
    );
    let vault = vault.canonicalize().unwrap_or(vault);
    let mode = Mode::Lexical;
    let (items, skipped) = known_items(&vault);
    assert!(!items.is_empty(), "no known-item queries in {FOLDERS:?}");
    // The app's prefilter: build the FTS index for this vault. In a test
    // build its root is a per-process temp folder, so the live SearchIndex
    // is never opened; a ranking change made inside the index (OB12's
    // BM25) shows here. That folder holds note bodies: `_search_root`
    // deletes it when the run ends, also on a panic.
    let _search_root = SearchIndexRoot::new();
    crate::vault_search_index::engage(&vault);
    let outcomes = run(&vault, mode, &items, None);
    assert!(
        outcomes.iter().all(|o| o.prefilter == "fts"),
        "the eval should run on the FTS prefilter, as the app does"
    );
    // The determinism check: the same queries again, ranks compared (ms
    // aside). Recorded, not asserted, and never naming a query.
    let lexical_again = run(&vault, mode, &items, None);
    let mut determinism = json!({
        "known_item_lexical": ranks_of(&outcomes) == ranks_of(&lexical_again),
        "known_item_blended": null,
    });
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut json = report(mode, &outcomes, &skipped, commit(), unix);
    json["misses"] = miss_report(&vault, &items, &outcomes);
    // OF01b: the semantic half (its snapshot is deleted inside setup) ...
    let ready = match semantic_setup() {
        Ok((ready, facts, snapshot)) => {
            let texts: Vec<&str> = items.iter().map(|i| i.query.as_str()).collect();
            let embedded = {
                let _quiet = QuietPanics::install();
                embed_all(&texts, &|t: &str| ready.embedder.query(t))
            };
            let qvecs = embedded.vecs.as_slice();
            let mut semantic = semantic_summary(&run_semantic(&ready.index, &items, qvecs));
            semantic["index"] = facts;
            semantic["snapshot"] = snapshot;
            semantic["fusion"] = fusion();
            semantic["embed_ms"] = json!({
                "p50": percentile(&embedded.ms, 50.0),
                "p95": percentile(&embedded.ms, 95.0),
            });
            semantic["embed_panicked"] = embedded.panicked.into();
            json["semantic"] = semantic;
            let input = Some((&ready.index, qvecs));
            let blended = run(&vault, Mode::Blended, &items, input);
            let blended_again = run(&vault, Mode::Blended, &items, input);
            determinism["known_item_blended"] =
                (ranks_of(&blended) == ranks_of(&blended_again)).into();
            json["blended"] = summary(&blended);
            Some(ready)
        }
        Err(skipped) => {
            json["semantic"] = skipped;
            json["blended"] = json!({ "skipped": "no semantic half" });
            None
        }
    };
    // ... and the real-query replay.
    json["replay"] = replay(&vault, ready.as_ref(), unix);
    for (pass, agreed) in [
        (
            "replay_lexical",
            json["replay"]["deterministic"]["lexical"].clone(),
        ),
        (
            "replay_blended",
            json["replay"]["deterministic"]["blended"].clone(),
        ),
    ] {
        determinism[pass] = agreed;
    }
    // Every pass that ran agreed with its rerun (a skipped one is `null`).
    let deterministic = determinism
        .as_object()
        .is_some_and(|m| m.values().all(|v| v.as_bool() != Some(false)));
    json["determinism"] = determinism;
    json["deterministic"] = deterministic.into();
    let out = env_nonempty("HOLLOW_RETRIEVAL_EVAL_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("retrieval-eval-{unix}-{}.json", std::process::id()))
        });
    std::fs::write(&out, serde_json::to_string_pretty(&json).unwrap())
        .expect("write the retrieval eval report");
    let t = &json["total"];
    eprintln!(
        "retrieval_eval {}: n={} recall@8={} recall@24={} mrr={} ndcg@8={} evidence={} ms p50={} p95={} -> {}",
        mode.name(),
        t["n"],
        t["recall_at_8"],
        t["recall_at_24"],
        t["mrr"],
        t["ndcg_at_8"],
        t["evidence_recall"],
        t["broker_ms"]["p50"],
        t["broker_ms"]["p95"],
        out.display()
    );
    let s = &json["semantic"]["total"];
    let b = &json["blended"]["total"];
    // Sizes and SQLite's verdicts only: no path.
    let es = &json["semantic"]["snapshot"];
    eprintln!(
        "retrieval_eval semantic: schema={} embed wal_file_bytes={} quick_check={} recall@8={} recall@24={} mrr={} | blended: recall@8={} recall@24={} mrr={} | {}",
        json["semantic"]["index"]["schema_version"],
        es["live_wal_file_bytes"],
        es["snapshot_quick_check"],
        s["recall_at_8"],
        s["recall_at_24"],
        s["mrr"],
        b["recall_at_8"],
        b["recall_at_24"],
        b["mrr"],
        json["semantic"]["skipped"],
    );
    let r = &json["replay"];
    let ss = &r["sessions_snapshot"];
    eprintln!(
        "retrieval_eval replay: sessions wal_file_bytes={} quick_check={} runs={} request_missing={} drift={} panicked={}/{} lexical run_mrr_first={} target recall@8={} mrr={} | blended run_mrr_first={} target recall@8={} mrr={} | {}",
        ss["live_wal_file_bytes"],
        ss["snapshot_quick_check"],
        r["runs"]["replayed"],
        r["runs"]["request_missing"],
        r["kept"]["drift"],
        r["lexical"]["panicked"],
        r["blended"]["panicked"],
        r["lexical"]["run_mrr_first"],
        r["lexical"]["target_recall_at_8"],
        r["lexical"]["target_mrr"],
        r["blended"]["run_mrr_first"],
        r["blended"]["target_recall_at_8"],
        r["blended"]["target_mrr"],
        r["skipped"],
    );
    eprintln!(
        "retrieval_eval deterministic={} {}",
        json["deterministic"], json["determinism"]
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn vault() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::Builder::new()
            .prefix("retrieval-eval-")
            .tempdir()
            .unwrap();
        let v = t.path().canonicalize().unwrap().join("Vault");
        fs::create_dir_all(&v).unwrap();
        (t, v)
    }

    fn passage(path: &str, line: usize) -> crate::directory_connections::Passage {
        crate::directory_connections::Passage {
            path: path.into(),
            line,
            excerpt: "x".into(),
            source: "note".into(),
            coverage: "excerpt".into(),
            score: 1,
            semantic: None,
            lifecycle: None,
            note: None,
            current: None,
        }
    }

    #[test]
    fn metrics_score_a_first_a_third_and_a_miss_exactly() {
        let ranks = [Some(1), Some(3), None];
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        assert!(close(recall_at(&ranks, 1), 1.0 / 3.0));
        assert!(close(recall_at(&ranks, 2), 1.0 / 3.0));
        assert!(close(recall_at(&ranks, 8), 2.0 / 3.0));
        assert!(close(recall_at(&ranks, 24), 2.0 / 3.0));
        // (1 + 1/3 + 0) / 3
        assert!(close(mrr(&ranks), 4.0 / 9.0));
        // (1/log2(2) + 1/log2(4) + 0) / 3 = (1 + 0.5) / 3
        assert!(close(ndcg_at(&ranks, 8), 0.5));
        // Rank 3 falls outside nDCG@2.
        assert!(close(ndcg_at(&ranks, 2), 1.0 / 3.0));
        // Rank 10 counts for MRR, not for nDCG@8 or recall@8.
        assert!(close(ndcg_at(&[Some(10)], 8), 0.0));
        assert!(close(recall_at(&[Some(10)], 8), 0.0));
        assert!(close(mrr(&[Some(10)]), 0.1));
        assert_eq!(recall_at(&[], 8), 0.0);
        assert_eq!(mrr(&[]), 0.0);
        assert_eq!(ndcg_at(&[], 8), 0.0);
        assert_eq!(percentile(&[5, 1, 3, 2, 4], 50.0), 3);
        assert_eq!(percentile(&[5, 1, 3, 2, 4], 95.0), 5);
        assert_eq!(percentile(&[], 50.0), 0);
    }

    #[test]
    fn a_rank_counts_distinct_files_in_pool_order() {
        let v = Path::new("/v");
        let pool = [
            passage("/v/library/a.md", 1),
            passage("/v/library/a.md", 4),
            passage("/v/tasks/b.md", 2),
            passage("/projects/repo/README.md", 1),
            passage("/v/learnings/c.md", 3),
        ];
        assert_eq!(note_rank(v, &pool, "library/a.md"), Some(1));
        assert_eq!(note_rank(v, &pool, "tasks/b.md"), Some(2));
        // A connected project file is a file in the ranking.
        assert_eq!(note_rank(v, &pool, "learnings/c.md"), Some(4));
        assert_eq!(note_rank(v, &pool, "notes/d.md"), None);
    }

    #[test]
    fn queries_embed_titles_and_skip_short_shared_restricted_and_other_folders() {
        let (_t, v) = vault();
        put(
            &v,
            "library/zebra.md",
            "---\ntitle: Zebra migration plan\n---\nBody.\n",
        );
        put(
            &v,
            "tasks/fix-api-authentication.md",
            "Tokens expire early.\n",
        );
        put(
            &v,
            "learnings/2026-10-01-kestrel.md",
            "---\ntitle: Kestrel calibration drift\n---\nBody.\n",
        );
        // No retrieval term: stopwords only.
        put(&v, "library/the-and.md", "Body.\n");
        // The same terms in another order: no single right answer.
        put(&v, "library/plan-a.md", "---\ntitle: Orchid rollout\n---\n");
        put(&v, "tasks/plan-b.md", "---\ntitle: rollout orchid\n---\n");
        // Never read, counted.
        put(
            &v,
            "library/restricted/secret.md",
            "---\ntitle: Secret salary bands\n---\n",
        );
        put(
            &v,
            "library/_index.md",
            "---\ntitle: Library index page\n---\n",
        );
        put(&v, "library/page.html", "<h1>Html page title</h1>");
        put(&v, "notes/other.md", "---\ntitle: Other folder note\n---\n");
        let (items, skipped) = known_items(&v);
        let got: Vec<(&str, &str, &str)> = items
            .iter()
            .map(|i| (i.folder, i.rel.as_str(), i.query.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    "library",
                    "library/zebra.md",
                    query_for("Zebra migration plan").as_str()
                ),
                (
                    "tasks",
                    "tasks/fix-api-authentication.md",
                    query_for("fix-api-authentication").as_str()
                ),
                (
                    "learnings",
                    "learnings/2026-10-01-kestrel.md",
                    query_for("Kestrel calibration drift").as_str()
                ),
            ]
        );
        assert_eq!(
            skipped,
            Skipped {
                restricted: 1,
                unreadable: 0,
                short_title: 1,
                duplicate_title: 2,
                capped: 0,
            }
        );
        assert_eq!(
            query_for(" Zebra migration plan "),
            "before tomorrow's meeting could you pull together everything we have on Zebra \
             migration plan including notes, tasks, decisions and open questions so I can review it"
        );
        assert_eq!(
            title_terms("Fix the API authentication"),
            vec!["fix", "api", "authentication"]
        );
    }

    #[test]
    fn caps_keep_evenly_spaced_items_in_path_order() {
        let v: Vec<usize> = (0..10).collect();
        assert_eq!(spaced(v.clone(), 4), vec![0, 2, 5, 7]);
        assert_eq!(spaced(v.clone(), 10), v);
        assert_eq!(spaced(v, 20).len(), 10);
        let (_t, vault) = vault();
        // Three times as many tasks as library notes: the cap keeps that
        // proportion instead of a fixed share per folder.
        for i in 0..(MAX_ITEMS + 100) {
            let folder = if i % 4 == 0 { "library" } else { "tasks" };
            put(
                &vault,
                &format!("{folder}/item-{i:03}.md"),
                &format!("---\ntitle: Heron item {i:03}\n---\n"),
            );
        }
        let (items, skipped) = known_items(&vault);
        assert_eq!(items.len(), MAX_ITEMS);
        assert_eq!(skipped.capped, 100);
        assert_eq!(items[0].rel, "library/item-000.md");
        let library = items.iter().filter(|i| i.folder == "library").count();
        assert_eq!(library, MAX_ITEMS / 4);
        // Folder order (FOLDERS), then path order inside a folder.
        assert!(items[..library].iter().all(|i| i.folder == "library"));
        assert!(items
            .windows(2)
            .all(|w| w[0].folder != w[1].folder || w[0].rel < w[1].rel));
    }

    fn miss_outcome(rank: Option<usize>, in_evidence: bool, stopped: Option<&str>) -> Outcome {
        Outcome {
            folder: "library",
            rank,
            in_evidence,
            ms: 1,
            prefilter: "linear".into(),
            stopped: stopped.map(str::to_string),
            semantic: "off".into(),
        }
    }

    /// RC2: four planted failures each report their own stage, and a note
    /// that reached the agent reports none.
    #[test]
    fn each_planted_miss_reports_its_stage() {
        let (_t, v) = vault();
        put(&v, "library/Kept note.md", "# Kept\nbody\n");
        put(&v, "library/restricted/Secret plan.md", "# Secret\nbody\n");
        put(&v, "inbox/Draft.md", "# Draft\nbody\n");
        put(&v, "library/.trash/Old.md", "# Old\nbody\n");
        put(&v, "briefings/Brief.html", "<p>brief</p>");
        // Not indexed: the rule that hides it is named, for each way.
        let hit = miss_outcome(Some(1), true, None);
        for (rel, rule) in [
            ("library/restricted/Secret plan.md", "restricted"),
            ("inbox/Draft.md", "skipped folder"),
            ("library/.trash/Old.md", "hidden"),
            ("library/Gone.md", "missing"),
            ("library/notes.txt", "note file"),
        ] {
            let stage = diagnose_miss(&v, rel, &hit, None).expect(rel);
            assert_eq!(stage.name(), "not_indexed", "{rel}");
            assert!(stage.why().contains(rule), "{rel}: {}", stage.why());
        }
        // An indexed html note that reached the agent is no miss.
        assert_eq!(diagnose_miss(&v, "briefings/Brief.html", &hit, None), None);
        let big = v.join("library/Big.md");
        fs::write(
            &big,
            vec![b'x'; crate::vault_search::RETRIEVAL_MAX_NOTE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(diagnose_miss(&v, "library/Big.md", &hit, None)
            .unwrap()
            .why()
            .contains("size limit"));
        // Indexed but outranked: not in the pool, scan ran to the end.
        let kept = "library/Kept note.md";
        assert_eq!(
            diagnose_miss(&v, kept, &miss_outcome(None, false, None), None),
            Some(MissStage::Outranked)
        );
        // Ranked but cut by a budget: past the evidence cap, or the scan stopped.
        let cut = diagnose_miss(&v, kept, &miss_outcome(Some(11), false, None), None).unwrap();
        assert_eq!(cut.name(), "cut_by_budget");
        assert!(cut.why().contains("ranked 11"), "{}", cut.why());
        let stopped = diagnose_miss(
            &v,
            kept,
            &miss_outcome(None, false, Some("read budget")),
            None,
        )
        .unwrap();
        assert_eq!(stopped.name(), "cut_by_budget");
        assert!(stopped.why().contains("read budget"));
        // Delivered but unused: only when the caller knows the answer.
        assert_eq!(
            diagnose_miss(&v, kept, &hit, Some(false)),
            Some(MissStage::DeliveredUnused)
        );
        assert_eq!(diagnose_miss(&v, kept, &hit, Some(true)), None);
        assert_eq!(diagnose_miss(&v, kept, &hit, None), None);
    }

    #[test]
    fn the_miss_report_counts_stages_and_writes_the_doctor_line() {
        let (_t, v) = vault();
        put(&v, "library/A note.md", "# A\nbody\n");
        put(&v, "library/B note.md", "# B\nbody\n");
        put(&v, "inbox/C.md", "# C\nbody\n");
        let item = |rel: &str| Item {
            folder: "library",
            rel: rel.into(),
            query: "q".into(),
        };
        let items = [
            item("library/A note.md"),
            item("library/B note.md"),
            item("inbox/C.md"),
        ];
        let outcomes = [
            miss_outcome(Some(1), true, None),
            miss_outcome(None, false, None),
            miss_outcome(Some(2), true, None),
        ];
        let json = miss_report(&v, &items, &outcomes);
        assert_eq!(json["total"], 2);
        assert_eq!(json["of"], 3);
        assert_eq!(json["stages"]["outranked"], 1);
        assert_eq!(json["stages"]["not_indexed"], 1);
        assert_eq!(
            json["doctor_line"],
            "Recall missed 2 of 3 notes: 1 not indexed, 1 outranked."
        );
        assert!(
            !json.to_string().contains("library/"),
            "counts only, no path"
        );
        let all = miss_report(&v, &items[..1], &outcomes[..1]);
        assert_eq!(all["doctor_line"], "Recall found all 1 known notes.");
    }

    #[test]
    fn the_broker_finds_a_titled_note_and_the_report_has_every_metric() {
        let (_t, v) = vault();
        put(
            &v,
            "library/Zebra migration plan.md",
            "---\ntitle: Zebra migration plan\n---\nCutover steps for the zebra cluster.\n",
        );
        put(
            &v,
            "tasks/Heron budget review.md",
            "---\ntitle: Heron budget review\n---\nHeron budget numbers are due Friday.\n",
        );
        put(&v, "notes/unrelated.md", "# Unrelated\nLunch order.\n");
        let (items, skipped) = known_items(&v);
        assert_eq!(items.len(), 2);
        let outcomes = run(&v, Mode::Lexical, &items, None);
        assert!(outcomes.iter().all(|o| o.rank == Some(1)), "{outcomes:?}");
        assert!(outcomes.iter().all(|o| o.in_evidence));
        assert!(outcomes.iter().all(|o| o.prefilter == "linear"));
        let json = report(Mode::Lexical, &outcomes, &skipped, None, 7);
        assert_eq!(json["mode"], "lexical");
        assert_eq!(json["total"]["n"], 2);
        assert_eq!(json["total"]["recall_at_8"], 1.0);
        assert_eq!(json["total"]["recall_at_24"], 1.0);
        assert_eq!(json["total"]["mrr"], 1.0);
        assert_eq!(json["total"]["ndcg_at_8"], 1.0);
        assert_eq!(json["per_folder"]["library"]["n"], 1);
        assert_eq!(json["per_folder"]["learnings"]["n"], 0);
        assert!(json["commit"].is_null());
        // Aggregates only: no path or title reaches the report.
        let text = json.to_string();
        assert!(!text.contains("Zebra") && !text.contains("Heron"));
    }

    // ── OF01b ─────────────────────────────────────────────────────────────

    /// The embed index's tables at schema `version` (2: no
    /// `in_frontmatter`), in a fresh SQLite file under `dir`.
    fn embed_db(dir: &Path, version: i64) -> (PathBuf, Connection) {
        let path = dir.join(format!("embed-v{version}-{}.db", std::process::id()));
        let conn = Connection::open(&path).unwrap();
        let frontmatter = if version >= 3 {
            "in_frontmatter INTEGER NOT NULL DEFAULT 0,"
        } else {
            ""
        };
        conn.execute_batch(&format!(
            "CREATE TABLE meta(path TEXT PRIMARY KEY, mtime TEXT NOT NULL, size INTEGER NOT NULL, \
               title TEXT NOT NULL, kind TEXT NOT NULL, chunk_count INTEGER NOT NULL); \
             CREATE TABLE chunks(path TEXT NOT NULL, ord INTEGER NOT NULL, heading TEXT, \
               byte_start INTEGER NOT NULL, byte_end INTEGER NOT NULL, text TEXT NOT NULL, \
               {frontmatter} PRIMARY KEY(path, ord)); \
             CREATE TABLE vectors(path TEXT NOT NULL, ord INTEGER NOT NULL, dim INTEGER NOT NULL, \
               vec BLOB NOT NULL, PRIMARY KEY(path, ord)); \
             PRAGMA user_version = {version};"
        ))
        .unwrap();
        (path, conn)
    }

    /// One chunk of `path` with vector `vec` (empty: a tombstone, `dim = 0`).
    fn add_chunk(conn: &Connection, path: &str, ord: i64, body_len: i64, vec: &[f32]) {
        conn.execute(
            "INSERT OR IGNORE INTO meta(path, mtime, size, title, kind, chunk_count) \
             VALUES (?1, '2026-10-17T00:00:00Z', 10, 'Title', 'note', 1)",
            [path],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks(path, ord, heading, byte_start, byte_end, text) \
             VALUES (?1, ?2, NULL, 0, ?3, 'chunk text')",
            rusqlite::params![path, ord, body_len],
        )
        .unwrap();
        let dim = vec.len() as i64;
        let blob: Vec<u8> = vec.iter().flat_map(|f| f.to_le_bytes()).collect();
        conn.execute(
            "INSERT INTO vectors(path, ord, dim, vec) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![path, ord, dim, blob],
        )
        .unwrap();
    }

    fn outcome(folder: &'static str, rank: Option<usize>, ms: u64) -> Outcome {
        Outcome {
            folder,
            rank,
            in_evidence: rank.is_some_and(|r| r <= 3),
            ms,
            prefilter: "fts".into(),
            stopped: None,
            semantic: "off".into(),
        }
    }

    /// The metric functions are pure over fixed outcomes and vector ties
    /// break by row. Not the determinism check: that reruns the broker over
    /// the real vault in `real_vault_known_item_retrieval` (`deterministic`).
    #[test]
    fn metrics_of_fixed_outcomes_are_pure_and_vector_ties_break_by_row() {
        let once = || {
            let outcomes = vec![
                outcome("library", Some(1), 40),
                outcome("library", None, 90),
                outcome("tasks", Some(5), 12),
                outcome("learnings", Some(30), 7),
            ];
            let semantic = vec![
                SemanticOutcome {
                    folder: "library",
                    rank: Some(2),
                    reaches_blend: true,
                    embedded: true,
                },
                SemanticOutcome {
                    folder: "tasks",
                    rank: None,
                    reaches_blend: false,
                    embedded: false,
                },
            ];
            let replayed = vec![
                Some(ReplayOutcome {
                    ranks: vec![Some(3), None, Some(9)],
                    ms: 20,
                }),
                Some(ReplayOutcome {
                    ranks: vec![None],
                    ms: 31,
                }),
                // A run whose broker call panicked: counted, not scored.
                None,
            ];
            // Ties in the vector ranking break by row, so the order is total.
            let index = VectorIndex {
                rows: ["library/a.md", "library/b.md", "tasks/c.md"]
                    .iter()
                    .map(|p| VectorRow {
                        path: (*p).into(),
                        ord: 0,
                        kind: "note".into(),
                        mtime: String::new(),
                        byte_start: 0,
                        byte_end: 1,
                        in_frontmatter: false,
                        vec: vec![0.6, 0.8],
                    })
                    .collect(),
                ..Default::default()
            };
            let scored = index.scored(&[0.6, 0.8]);
            let ranking = index.ranking(&scored, POOL, &|_: &str| true);
            json!({
                "summary": summary(&outcomes),
                "semantic": semantic_summary(&semantic),
                "replay": replay_metrics(&replayed),
                "ranking": ranking,
                "hits": index.app_hits(&scored, 2).iter().map(|h| h.path.clone()).collect::<Vec<_>>(),
            })
            .to_string()
        };
        let (a, b) = (once(), once());
        assert_eq!(a, b);
        let v: Value = serde_json::from_str(&a).unwrap();
        assert_eq!(v["summary"]["total"]["recall_at_8"], 0.5);
        assert_eq!(v["semantic"]["total"]["mrr"], 0.25);
        // Targets: 3, miss, 9, miss -> recall@8 1/4; runs: best 3, miss.
        assert_eq!(v["replay"]["target_recall_at_8"], 0.25);
        assert_eq!(v["replay"]["run_any_at_24"], 0.5);
        assert_eq!(v["replay"]["runs"], 2);
        assert_eq!(v["replay"]["panicked"], 1);
        // Ceiling: 3 targets at best 1, 2, 3 and 1 at best 1:
        // (1 + 1/2 + 1/3 + 1) / 4.
        assert_eq!(v["replay"]["target_mrr_ceiling"], 0.7083);
        assert_eq!(v["hits"], json!(["library/a.md", "library/b.md"]));
        // The determinism comparison looks at ranks, not ms.
        let mut slower = vec![outcome("library", Some(1), 40), outcome("tasks", None, 9)];
        let first = ranks_of(&slower);
        slower[0].ms = 4_000;
        assert_eq!(ranks_of(&slower), first);
        slower[1].rank = Some(2);
        assert_ne!(ranks_of(&slower), first);
        // A panic is caught, its payload dropped.
        assert_eq!(quietly(|| 7), Some(7));
        assert_eq!(quietly(|| -> u8 { panic!("synthetic") }), None);
    }

    #[test]
    fn restricted_rows_are_dropped_from_a_synthetic_vectors_table() {
        let t = tempfile::Builder::new()
            .prefix("retrieval-eval-vectors-")
            .tempdir()
            .unwrap();
        let (_path, conn) = embed_db(t.path(), 3);
        add_chunk(&conn, "library/kestrel.md", 0, 5, &[1.0, 0.0]);
        add_chunk(&conn, "library/kestrel.md", 1, 5, &[0.0, 1.0]);
        add_chunk(&conn, "library/restricted/pay.md", 0, 5, &[1.0, 0.0]);
        add_chunk(&conn, "restricted/hr.md", 0, 5, &[1.0, 0.0]);
        add_chunk(&conn, "restricted/hr.md", 1, 5, &[1.0, 0.0]);
        add_chunk(&conn, "tasks/broken.md", 0, 5, &[]);
        let index = load_vectors(&conn).unwrap();
        assert_eq!(index.schema, 3);
        assert_eq!(index.vectors_total, 6);
        assert_eq!(index.restricted_rows, 3);
        assert_eq!(index.restricted_notes, 2);
        assert_eq!(index.tombstones, 1);
        assert_eq!(index.malformed, 0);
        let paths: Vec<(&str, usize)> = index
            .rows
            .iter()
            .map(|r| (r.path.as_str(), r.ord))
            .collect();
        assert_eq!(
            paths,
            vec![("library/kestrel.md", 0), ("library/kestrel.md", 1)]
        );
        // A restricted note can't answer even a query aimed straight at it.
        let scored = index.scored(&[1.0, 0.0]);
        let hits = index.app_hits(&scored, SEMANTIC_NOTES);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "library/kestrel.md");
        assert!((hits[0].score - 1.0).abs() < 1e-6);
        let facts = index_facts(&index).to_string();
        assert!(!facts.contains("restricted/"), "{facts}");
    }

    #[test]
    fn the_schema_version_is_read_from_user_version_or_the_columns() {
        let t = tempfile::Builder::new()
            .prefix("retrieval-eval-schema-")
            .tempdir()
            .unwrap();
        let (_p3, v3) = embed_db(t.path(), 3);
        assert_eq!(detect_schema(&v3).unwrap(), (3, "user_version"));
        let (_p2, v2) = embed_db(t.path(), 2);
        assert_eq!(detect_schema(&v2).unwrap(), (2, "user_version"));
        // A v2 index reads too: no frontmatter chunks.
        add_chunk(&v2, "library/heron.md", 0, 4, &[0.0, 1.0]);
        let index = load_vectors(&v2).unwrap();
        assert_eq!(index.schema, 2);
        assert_eq!(index.rows.len(), 1);
        assert!(!index.rows[0].in_frontmatter);
        assert_eq!(index.rows[0].byte_end, 4);
        // No user_version: told by the columns.
        v3.pragma_update(None, "user_version", 0).unwrap();
        assert_eq!(detect_schema(&v3).unwrap(), (3, "columns"));
        // A version that disagrees with its columns, or one this can't read.
        v2.pragma_update(None, "user_version", 3).unwrap();
        assert!(detect_schema(&v2).is_err());
        v3.pragma_update(None, "user_version", 4).unwrap();
        assert!(detect_schema(&v3).unwrap_err().contains("v4"));
        // A v1 chunk store has no vectors table.
        let v1 = Connection::open_in_memory().unwrap();
        v1.execute_batch(
            "CREATE TABLE chunks(path TEXT, ord INTEGER); CREATE TABLE meta(path TEXT); \
             PRAGMA user_version = 1;",
        )
        .unwrap();
        assert!(detect_schema(&v1).is_err());
        assert!(load_vectors(&v1).is_err());
    }

    /// An embed DB in WAL mode with one checkpointed row and `pending` more
    /// only in the WAL (autocheckpoint off), its writer still open, as the
    /// app holds the live index.
    fn wal_embed_db(dir: &Path, pending: usize) -> (PathBuf, Connection) {
        let (path, conn) = embed_db(dir, 3);
        add_chunk(&conn, "library/kestrel.md", 0, 5, &[1.0, 0.0]);
        let mode: String = conn
            .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        for i in 0..pending {
            add_chunk(&conn, &format!("tasks/heron-{i}.md"), 0, 5, &[0.0, 1.0]);
        }
        (path, conn)
    }

    /// The bytes of `db` and its `-wal` and `-shm` (`None`: absent).
    fn db_bytes(db: &Path) -> Vec<Option<Vec<u8>>> {
        ["", "-wal", "-shm"]
            .iter()
            .map(|suffix| {
                let mut p = db.as_os_str().to_owned();
                p.push(suffix);
                fs::read(PathBuf::from(p)).ok()
            })
            .collect()
    }

    #[test]
    fn reading_the_embed_index_leaves_a_live_wal_database_byte_identical() {
        let t = tempfile::Builder::new()
            .prefix("retrieval-eval-wal-")
            .tempdir()
            .unwrap();
        let (live, writer) = wal_embed_db(t.path(), 3);
        let before = db_bytes(&live);
        assert!(
            before.iter().all(Option::is_some),
            "db, -wal and -shm exist"
        );
        let wal_len = before[1].as_ref().unwrap().len();
        assert!(wal_len > 0, "frames are pending in the WAL");
        let (index, facts) = read_embed_index(&live).unwrap();
        assert_eq!(
            db_bytes(&live),
            before,
            "the live file, its -wal and -shm are untouched"
        );
        // The snapshot reads the main file only: the frames still in the
        // WAL are left out, and the WAL file's size is reported.
        assert_eq!(index.vectors_total, 1);
        assert_eq!(facts["live_wal_file_bytes"], wal_len as u64);
        assert_eq!(facts["snapshot_quick_check"], "ok");
        assert_eq!(facts["snapshot_deleted"], true);
        // Nothing was locked: the writer carries on.
        add_chunk(&writer, "tasks/after.md", 0, 5, &[0.0, 1.0]);
    }

    #[test]
    fn the_snapshot_folder_is_gone_after_load_vectors_succeeds_or_fails() {
        let t = tempfile::Builder::new()
            .prefix("retrieval-eval-snapdir-")
            .tempdir()
            .unwrap();
        let (good, _writer) = wal_embed_db(t.path(), 1);
        let ok = through_snapshot(&good, "embed", load_vectors).unwrap();
        assert_eq!(ok.value.as_ref().unwrap().rows.len(), 1);
        assert!(!ok.dir.exists());
        let name = ok.dir.file_name().unwrap().to_string_lossy().into_owned();
        let ours = format!("hollow-test-retrieval-eval-{}-", std::process::id());
        assert!(name.starts_with(&ours), "{name}");
        // No embed tables: `load_vectors` fails, the folder is gone all the same.
        let bad = t
            .path()
            .join(format!("not-embed-{}.db", std::process::id()));
        Connection::open(&bad)
            .unwrap()
            .execute_batch("CREATE TABLE notes(path TEXT);")
            .unwrap();
        let failed = through_snapshot(&bad, "embed", load_vectors).unwrap();
        assert!(failed.value.is_err());
        assert!(!failed.dir.exists());
        assert_ne!(ok.dir, failed.dir);
        // The half reports it as a skip, with the facts and no path.
        let skipped = read_embed_index(&bad).unwrap_err();
        assert!(
            skipped["skipped"].as_str().unwrap().contains("no chunks"),
            "{skipped}"
        );
        assert_eq!(skipped["snapshot"]["snapshot_deleted"], true);
        let text = skipped.to_string();
        assert!(!text.contains(t.path().to_str().unwrap()), "{text}");
    }

    #[test]
    fn a_snapshot_refuses_a_bad_temp_dir_and_creates_nothing_there() {
        let t = tempfile::Builder::new()
            .prefix("retrieval-eval-refuse-")
            .tempdir()
            .unwrap();
        let (live, _writer) = wal_embed_db(t.path(), 0);
        let made_here = |dir: &Path| {
            fs::read_dir(dir).is_ok_and(|entries| {
                entries.flatten().any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("hollow-test-retrieval-eval-")
                })
            })
        };
        // Inside this checkout.
        let checkout = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(Snapshot::take_in(checkout, &live, "embed").is_err());
        assert!(!made_here(checkout));
        // Under a `Worktrees` folder.
        let worktrees = t.path().join("Worktrees").join("lane");
        fs::create_dir_all(&worktrees).unwrap();
        assert!(Snapshot::take_in(&worktrees, &live, "embed").is_err());
        assert_eq!(fs::read_dir(&worktrees).unwrap().count(), 0);
        // Unresolvable: fail closed.
        let missing = t.path().join("missing");
        assert!(Snapshot::take_in(&missing, &live, "embed").is_err());
        assert!(!missing.exists());
        // The test's own temp dir is fine, and the folder goes with the guard.
        let snap = Snapshot::take_in(&std::env::temp_dir(), &live, "embed").unwrap();
        assert_eq!(snap.quick_check, "ok");
        let dir = snap.dir.clone();
        drop(snap);
        assert!(!dir.exists());
    }

    fn jev(path: &str, rank: u64, kept: bool) -> Value {
        json!({
            "path": path,
            "line": 3,
            "rank": rank,
            "localScore": 2,
            "jevScore": 0.75,
            "semanticScore": 0.66,
            "kept": kept,
            "basis": "jev",
            "reason": "Zebra cutover is the topic",
            "bytes": 10,
            "sha256": "x",
        })
    }

    #[test]
    fn replay_counts_drift_restricted_and_project_files_and_skips_them() {
        let (_t, v) = vault();
        put(&v, "library/zebra plan.md", "Zebra cutover.\n");
        put(&v, "tasks/heron.md", "Heron.\n");
        let manifest = json!({
            "runId": "run-1",
            "retrieval": {
                "engine": "jev",
                "candidates": [
                    jev("library/zebra plan.md", 1, true),
                    // The same note twice counts once.
                    jev("library/zebra plan.md", 2, true),
                    jev("tasks/heron.md", 3, false),
                    jev("library/gone.md", 4, true),
                    jev("library/restricted/pay.md", 5, true),
                    jev("/Users/someone/repo/README.md", 6, true),
                    jev("../outside.md", 7, true),
                    {"path": "notes/local.md", "rank": 8, "kept": true, "basis": "local"},
                    // In the vault, but the broker no longer considers them:
                    // their own skip, not a miss.
                    jev("me/profile.md", 9, true),
                    jev("workspaces/w/brief.md", 10, true),
                    // Jev's, but no boolean `kept`: counted, left out.
                    {"path": "library/odd.md", "rank": 11, "kept": "yes", "basis": "jev"},
                ],
            },
        });
        put(&v, "me/profile.md", "Me.\n");
        put(&v, "workspaces/w/brief.md", "Brief.\n");
        let run = jev_run(&manifest).unwrap();
        assert_eq!(run.run_id, "run-1");
        assert_eq!(run.candidates.len(), 9, "the local candidate is not Jev's");
        assert_eq!(run.malformed, 1);
        assert_eq!(run.query_basis, None);
        let mut skips = KeptSkips::default();
        let targets = replay_targets(&v, &run, &mut skips);
        assert_eq!(targets, vec!["library/zebra plan.md".to_string()]);
        assert_eq!(
            skips,
            KeptSkips {
                restricted: 1,
                project_file: 1,
                invalid: 1,
                ineligible: 2,
                drift: 1,
            }
        );
        // Only a run chat queried with its own request is replayed; a
        // manifest predating `queryBasis` counts as one.
        let mut wake = manifest.clone();
        wake["retrieval"]["queryBasis"] = "hostSubject".into();
        let wake = jev_run(&wake).unwrap();
        assert_eq!(wake.query_basis.as_deref(), Some("hostSubject"));
        assert!(!replays_basis(wake.query_basis.as_deref()));
        assert!(replays_basis(None));
        assert!(replays_basis(Some("request")));
        assert!(!replays_basis(Some("followup+prior")));
        assert!(!replays_basis(Some("skipped-host")));
        // Every Jev candidate malformed: told apart from "not Jev's".
        let odd = json!({"runId": "r", "retrieval": {"engine": "jev", "candidates": [
            {"path": "a.md", "rank": 1, "basis": "jev"},
        ]}});
        assert_eq!(jev_run(&odd), Err("malformed"));
        // Manifests Jev didn't label are told apart.
        assert_eq!(jev_run(&json!({"runId": "r"})), Err("no_candidates"));
        let local = json!({"runId": "r", "retrieval": {"engine": "local", "candidates": [jev("a.md", 1, true)]}});
        assert_eq!(jev_run(&local), Err("not_jev"));
        let no_id = json!({"retrieval": {"engine": "jev", "candidates": [jev("a.md", 1, true)]}});
        assert_eq!(jev_run(&no_id), Err("no_run_id"));
        // The broker finds the kept note for a request naming it.
        let items = vec![ReplayItem {
            request: "what did we decide about the zebra cutover plan".into(),
            targets,
        }];
        let out = run_replay(&v, Mode::Lexical, &items, None);
        assert_eq!(out[0].as_ref().unwrap().ranks, vec![Some(1)]);
        let m = replay_metrics(&out);
        assert_eq!(m["target_recall_at_8"], 1.0);
        assert_eq!(m["panicked"], 0);
        assert!(!m.to_string().contains("zebra"));
    }

    #[test]
    fn the_label_file_holds_no_path_or_text() {
        let manifest = json!({
            "runId": "run-7",
            "retrieval": {
                "engine": "jev",
                "candidates": [
                    jev("library/Zebra migration plan.md", 1, true),
                    jev("tasks/Heron budget review.md", 2, false),
                    jev("library/restricted/Salary bands.md", 3, true),
                ],
            },
        });
        let run = jev_run(&manifest).unwrap();
        let (labels, left_out) = label_file(&[run]);
        assert_eq!(left_out, 1);
        let text = serde_json::to_string_pretty(&labels).unwrap();
        for leak in [
            "Zebra", "Heron", "Salary", "library/", "tasks/", "cutover", ".md",
        ] {
            assert!(!text.contains(leak), "{leak} reached the label file");
        }
        let c = &labels["runs"][0]["candidates"];
        assert_eq!(c.as_array().unwrap().len(), 2);
        assert_eq!(
            c[0]["pathSha256"],
            sha256_hex("library/Zebra migration plan.md")
        );
        assert_eq!(c[0]["kept"], true);
        assert_eq!(c[1]["kept"], false);
        assert_eq!(c[0]["rank"], 1);
        assert_eq!(c[0]["jevScore"], 0.75);
        assert_eq!(labels["runs"][0]["runId"], "run-7");
    }

    #[test]
    fn blended_mode_lets_a_semantic_hit_into_the_broker_pool() {
        let (_t, v) = vault();
        let lexical_body = "Zebra migration cutover steps.\n";
        let semantic_body = "Calibration drift on the kestrel rig.\n";
        put(&v, "library/zebra.md", lexical_body);
        put(&v, "library/kestrel.md", semantic_body);
        let row = |path: &str, len: usize, vec: Vec<f32>| VectorRow {
            path: path.into(),
            ord: 0,
            kind: "note".into(),
            mtime: String::new(),
            byte_start: 0,
            byte_end: len,
            in_frontmatter: false,
            vec,
        };
        let index = VectorIndex {
            schema: 3,
            rows: vec![
                row("library/kestrel.md", semantic_body.len(), vec![1.0, 0.0]),
                row("library/zebra.md", lexical_body.len(), vec![0.0, 1.0]),
            ],
            ..Default::default()
        };
        let items = vec![Item {
            folder: "library",
            rel: "library/kestrel.md".into(),
            query: query_for("zebra migration"),
        }];
        let qvecs = vec![Some(vec![0.9f32, 0.1])];
        let lexical = run(&v, Mode::Lexical, &items, None);
        assert_eq!(
            lexical[0].rank, None,
            "no query word is in the kestrel note"
        );
        let blended = run(&v, Mode::Blended, &items, Some((&index, qvecs.as_slice())));
        assert!(blended[0].rank.is_some(), "{blended:?}");
        assert_eq!(blended[0].semantic, "blended");
        let semantic = run_semantic(&index, &items, &qvecs);
        assert_eq!(semantic[0].rank, Some(1));
        assert!(semantic[0].reaches_blend);
        // A query that couldn't be embedded is lexical only, and a miss.
        let none: Vec<Option<Vec<f32>>> = vec![None];
        let blind = run(&v, Mode::Blended, &items, Some((&index, none.as_slice())));
        assert_eq!(blind[0].rank, None);
        assert_eq!(blind[0].semantic, "unavailable");
        let unembedded = run_semantic(&index, &items, &none);
        assert!(!unembedded[0].embedded);
        assert_eq!(unembedded[0].rank, None);
    }
}
