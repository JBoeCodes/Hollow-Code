//! The Jev study runner (agent effectiveness E3; owner decision Q3: measure
//! all four Jev uses). A hidden subcommand of the normal binary:
//!
//! ```text
//! hollow jev-study <fixture-dir> <out-dir> (--endpoint <loopback url> | --typesafe)
//!                  [--max-calls 200] [--items N] [--condition <name>]...
//! ```
//!
//! For each judged fixture in `<fixture-dir>` (`scripts/agent-eval/jev-fixtures/`,
//! one per use) it runs every item through the **production** decision code
//! twice: once with Jev off (the local condition) and once with Jev pointed at
//! the chosen endpoint. Nothing is re-implemented here:
//!
//! | Condition | Production path |
//! |---|---|
//! | `connector-relevance` | `context_rank::select_connectors_via` with `jev::score_each_with` (one request of at most 24, local preselection, named connectors never sent) |
//! | `passage-rerank` | `context_rank::prioritize_passages_via` with `jev::score_with` (one request; a pool is at most 24) |
//! | `proposal-dedup` | the exact rules (`vault_tasks::slugify` for tasks before Jev, `vault_learning::statement_key` for learnings after it), then `jev_decide::check_traced` |
//!
//! (A fourth, `whiteboard-layout`, went with whiteboards on 2026-10-07; the
//! 2026-10-07 study measured no difference for it.)
//!
//! Each result records the decision, its latency, the outcome (`off` for
//! the local condition; `ok`, `timeout`, `malformed`, `error`, or
//! `not-sent` when production sends nothing for that item), the requests
//! made, and the cost: `0` with no request, otherwise `null` (TypeSafe
//! reports no per-call cost; unknown is never 0). A failed request keeps the
//! fallback decision production makes, so every result carries an answer.
//!
//! **It refuses to run** unless `--endpoint` is a loopback URL
//! (`instance::loopback_jev_url`: plain http to 127.0.0.1 or `[::1]`) or
//! `--typesafe` is given explicitly. A loopback run sends a placeholder key,
//! never the saved one. `--typesafe` reads this instance's saved Jev key
//! the way the app does (`jev::saved_key`; a loose binary or the everyday
//! app reads the everyday Keychain item) and never prints it. Only the
//! orchestrator runs `--typesafe`.
//!
//! **Bounded.** One request per item at most, never a retry (the production
//! transport: 4 s, 64 KiB, no redirects), sequential, and `--max-calls`
//! (default 200) stops the run before a request that would pass it; the
//! run's `run.json` says where it stopped. A request is counted in
//! `run.json` before it is sent (`inFlight` names its item until it
//! returns), and `--resume` starts from the larger of that count and the
//! arm files' own, so a kill mid-request never lets the cap be passed. The local condition runs first
//! for every fixture, so a stop only shortens the Jev condition.
//!
//! Only synthetic fixture text is ever sent, and only to the endpoint given.
//! The judged labels (`relevant`, `named`, `duplicateOf`, `adjacent`, …) are
//! read by `scripts/agent-eval/jev-study.mjs`, never by this runner, so they
//! can't reach Jev.
use crate::context_rank::{self, ConnectorCandidate, ConnectorQuery};
use crate::directory_connections::Passage;
use crate::jev::{self, Question};
use crate::jev_decide::{self, Record, Subject, Verdict};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The key a loopback run sends: never the saved one.
const LOOPBACK_KEY: &str = "personalev-study-loopback";
pub(crate) const DEFAULT_MAX_CALLS: u32 = 200;
const MAX_CALLS_LIMIT: u32 = 10_000;

const USAGE: &str = "usage: hollow jev-study <fixture-dir> <out-dir> (--endpoint <http://127.0.0.1:PORT/...> | --typesafe) [--max-calls 200] [--items N] [--condition <name>]... [--resume]";

/// The three uses, in the order they run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Condition {
    Connectors,
    Passages,
    Dedup,
}

impl Condition {
    pub(crate) const ALL: [Condition; 3] =
        [Condition::Connectors, Condition::Passages, Condition::Dedup];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Condition::Connectors => "connector-relevance",
            Condition::Passages => "passage-rerank",
            Condition::Dedup => "proposal-dedup",
        }
    }

    /// The local condition's arm name (`jev-study.mjs` `JEV_CONDITIONS`).
    pub(crate) fn local_arm(self) -> &'static str {
        match self {
            Condition::Connectors | Condition::Passages => "local",
            Condition::Dedup => "exact",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Condition::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// Where the Jev condition's requests go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A loopback fake (literal 127.0.0.1 or `[::1]`).
    Loopback(String),
    /// TypeSafe, with this instance's saved key. Only on `--typesafe`.
    Typesafe,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Options {
    pub fixtures: PathBuf,
    pub out: PathBuf,
    pub target: Target,
    pub max_calls: u32,
    /// The first N items of each fixture (a quick run).
    pub items: Option<usize>,
    pub conditions: Vec<Condition>,
    /// Continue the study already in `out` (its `run.json`): finished Jev
    /// arms are kept, a stopped or interrupted one goes on from its first
    /// unanswered item, and `max_calls` caps the study's total.
    pub resume: bool,
}

fn number(flag: &str, value: Option<&String>, min: u64, max: u64) -> Result<u64, String> {
    let raw = value.ok_or_else(|| format!("{flag} needs a value"))?;
    raw.parse::<u64>()
        .ok()
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| format!("{flag} must be a whole number from {min} to {max}, not {raw:?}"))
}

/// The command line after `jev-study`. Refuses a run with no explicit
/// target, a non-loopback `--endpoint`, or both targets.
pub(crate) fn parse(args: &[String]) -> Result<Options, String> {
    let mut positional = Vec::new();
    let mut endpoint = None;
    let mut typesafe = false;
    let mut max_calls = DEFAULT_MAX_CALLS;
    let mut items = None;
    let mut conditions = Vec::new();
    let mut resume = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--resume" => {
                resume = true;
                i += 1;
            }
            "--endpoint" => {
                let raw = args.get(i + 1).ok_or("--endpoint needs a value")?;
                endpoint = Some(crate::instance::loopback_jev_url(raw).ok_or_else(|| {
                    format!("--endpoint must be plain http to the literal loopback IP 127.0.0.1 or [::1] (a fake Jev), not {raw:?}; to call TypeSafe, say --typesafe")
                })?);
                i += 2;
            }
            "--typesafe" => {
                typesafe = true;
                i += 1;
            }
            "--max-calls" => {
                max_calls = number(arg, args.get(i + 1), 1, MAX_CALLS_LIMIT.into())? as u32;
                i += 2;
            }
            "--items" => {
                items = Some(number(arg, args.get(i + 1), 1, 1000)? as usize);
                i += 2;
            }
            "--condition" => {
                let name = args.get(i + 1).ok_or("--condition needs a value")?;
                let c = Condition::parse(name).ok_or_else(|| {
                    format!(
                        "--condition must be one of {}",
                        Condition::ALL.map(Condition::name).join(", ")
                    )
                })?;
                if !conditions.contains(&c) {
                    conditions.push(c);
                }
                i += 2;
            }
            flag if flag.starts_with("--") => return Err(format!("unknown flag {flag}")),
            _ => {
                positional.push(args[i].clone());
                i += 1;
            }
        }
    }
    let [fixtures, out] = <[String; 2]>::try_from(positional)
        .map_err(|_| "give a fixture folder and an output folder".to_string())?;
    let target = match (endpoint, typesafe) {
        (Some(_), true) => return Err("give --endpoint or --typesafe, not both".into()),
        (Some(url), false) => Target::Loopback(url),
        (None, true) => Target::Typesafe,
        (None, false) => return Err("jev-study calls Jev only when told where: --endpoint <loopback url> for a fake, or --typesafe for TypeSafe itself (billed to the saved key's account)".into()),
    };
    if conditions.is_empty() {
        conditions = Condition::ALL.to_vec();
    }
    conditions.sort_by_key(|c| Condition::ALL.iter().position(|x| x == c));
    Ok(Options {
        fixtures: fixtures.into(),
        out: out.into(),
        target,
        max_calls,
        items,
        conditions,
        resume,
    })
}

/// `hollow jev-study …`: the exit code. 0 complete, 3 stopped at
/// `--max-calls`, 1 a failure while running, 2 refused.
pub fn main(args: Vec<String>) -> i32 {
    let opts = match parse(&args) {
        Ok(opts) => opts,
        Err(e) => {
            eprintln!("jev-study: {e}\n{USAGE}");
            return 2;
        }
    };
    let (key, endpoint) = match &opts.target {
        Target::Loopback(url) => (LOOPBACK_KEY.to_string(), url.clone()),
        Target::Typesafe => match jev::saved_key() {
            Some(key) => (key, jev::TYPESAFE_ENDPOINT.to_string()),
            None => {
                eprintln!("jev-study: --typesafe needs a saved Jev key in this instance's Keychain item (Settings → Accounts); none was found");
                return 2;
            }
        },
    };
    match tauri::async_runtime::block_on(run(&opts, &key, &endpoint)) {
        Ok(summary) => {
            println!("{}", summary.line());
            if summary.stopped {
                3
            } else {
                0
            }
        }
        Err(e) => {
            eprintln!("jev-study: {e}");
            1
        }
    }
}

/// What a run did, for the command's one line of output.
pub(crate) struct Summary {
    pub calls: u32,
    pub stopped: bool,
    pub out: PathBuf,
}

impl Summary {
    fn line(&self) -> String {
        format!(
            "jev-study: {} Jev request(s){}; results in {}",
            self.calls,
            if self.stopped {
                ", stopped at --max-calls (see run.json)"
            } else {
                ""
            },
            self.out.display()
        )
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn millis(started: Instant) -> f64 {
    (started.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
}

fn text<'a>(v: &'a Value, key: &str, at: &str) -> Result<&'a str, String> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("{at}: {key} must be text"))
}

fn list<'a>(v: &'a Value, key: &str, at: &str) -> Result<&'a Vec<Value>, String> {
    v[key]
        .as_array()
        .filter(|l| !l.is_empty())
        .ok_or_else(|| format!("{at}: {key} must be a non-empty list"))
}

/// Ids are labels in the output only, never paths.
fn ids(items: &[Value], at: &str) -> Result<Vec<String>, String> {
    let mut seen = std::collections::BTreeSet::new();
    items
        .iter()
        .map(|x| {
            let id = text(x, "id", at)?.to_string();
            if id.len() > 64 || !seen.insert(id.clone()) {
                return Err(format!("{at}: id {id:?} is too long or appears twice"));
            }
            Ok(id)
        })
        .collect()
}

/// A judged fixture: its items (the first `limit`) and the file's digest.
pub(crate) struct Fixture {
    pub items: Vec<Value>,
    pub sha256: String,
}

pub(crate) fn load(
    dir: &Path,
    condition: Condition,
    limit: Option<usize>,
) -> Result<Fixture, String> {
    let path = dir.join(format!("{}.json", condition.name()));
    let raw = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let json: Value =
        serde_json::from_slice(&raw).map_err(|e| format!("parse {}: {e}", path.display()))?;
    if json["schema"] != 1 || json["condition"] != condition.name() {
        return Err(format!(
            "{} is not a schema 1 {} fixture",
            path.display(),
            condition.name()
        ));
    }
    let mut items = list(&json, "items", condition.name())?.clone();
    ids(&items, condition.name())?;
    if let Some(n) = limit {
        items.truncate(n);
    }
    for item in &items {
        check_item(condition, item)?;
    }
    Ok(Fixture {
        items,
        sha256: format!("{:x}", Sha256::digest(&raw)),
    })
}

/// Refuse what production would never be given, before any request.
fn check_item(condition: Condition, item: &Value) -> Result<(), String> {
    let at = item["id"].as_str().unwrap_or("?").to_string();
    match condition {
        Condition::Connectors => {
            text(item, "request", &at)?;
            let candidates = list(item, "candidates", &at)?;
            ids(candidates, &at)?;
            if candidates.len() > crate::connectors::MAX_CONNECTORS {
                return Err(format!(
                    "{at}: at most {} connector candidates (the library limit)",
                    crate::connectors::MAX_CONNECTORS
                ));
            }
            for c in candidates {
                text(c, "description", &at)?;
            }
        }
        Condition::Passages => {
            text(item, "request", &at)?;
            let passages = list(item, "passages", &at)?;
            ids(passages, &at)?;
            if passages.len() > jev::MAX_CANDIDATES {
                return Err(format!(
                    "{at}: at most {} passages (the pool the reranker sees)",
                    jev::MAX_CANDIDATES
                ));
            }
            for p in passages {
                text(p, "title", &at)?;
                text(p, "excerpt", &at)?;
            }
        }
        Condition::Dedup => {
            if !matches!(item["subject"].as_str(), Some("task" | "learning")) {
                return Err(format!("{at}: subject must be task or learning"));
            }
            text(&item["proposal"], "title", &at)?;
            let existing = list(item, "existing", &at)?;
            ids(existing, &at)?;
            for e in existing {
                text(e, "title", &at)?;
            }
        }
    }
    Ok(())
}

/// Jev's side of one item: `None` is the local condition.
type Jev<'a> = Option<(&'a str, &'a str)>;

/// The outcome of one item's (at most one) request.
fn outcome(jev: Jev<'_>, sent: Option<&Result<(), String>>) -> String {
    match (jev, sent) {
        (None, _) => "off".into(),
        (Some(_), None) => "not-sent".into(),
        (Some(_), Some(Ok(()))) => "ok".into(),
        (Some(_), Some(Err(e))) => jev::outcome_of(e).into(),
    }
}

/// The outcome from the production record of the call (`JevUse`, `None`
/// when nothing was sent).
fn used(jev: Jev<'_>, call: Option<&crate::run_manifest::JevUse>) -> String {
    match (jev, call) {
        (None, _) => "off".into(),
        (Some(_), None) => "not-sent".into(),
        (Some(_), Some(u)) => u.outcome.clone(),
    }
}

fn result(id: &str, outcome: String, started: Instant, calls: u32, answer: Value) -> Value {
    let mut out = json!({
        "id": id,
        "outcome": outcome,
        "latencyMs": millis(started),
        "calls": calls,
        // No request: a known 0. A request: TypeSafe reports no cost.
        "costUsd": if calls == 0 { json!(0) } else { Value::Null },
    });
    if let (Some(out), Some(answer)) = (out.as_object_mut(), answer.as_object()) {
        out.extend(answer.clone());
    }
    out
}

fn or_empty(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

async fn connector_item(item: &Value, jev: Jev<'_>) -> Value {
    let id = or_empty(item, "id");
    let request = or_empty(item, "request");
    let owned: Vec<(String, String, String)> = item["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let id = or_empty(c, "id");
            let name = c["name"]
                .as_str()
                .map_or_else(|| id.clone(), str::to_string);
            (id, name, or_empty(c, "description"))
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
    let query = ConnectorQuery::plain(&request);
    let calls = Cell::new(0u32);
    let started = Instant::now();
    let selection = context_rank::select_connectors_via(&query, &candidates, |text, items| {
        if jev.is_some() {
            calls.set(calls.get() + 1);
        }
        async move {
            let (key, endpoint) = jev?;
            Some(jev::score_each_with(key, endpoint, text, &items, Question::Tool).await)
        }
    })
    .await;
    let outcome = used(jev, selection.jev.as_ref());
    result(
        &id,
        outcome,
        started,
        calls.get(),
        json!({
            "engine": selection.engine,
            "note": selection.note,
            "decisions": selection.decisions,
        }),
    )
}

async fn passage_item(item: &Value, jev: Jev<'_>) -> Value {
    let id = or_empty(item, "id");
    let request = crate::sessions::strip_injected_context(&or_empty(item, "request"));
    let given: Vec<&Value> = item["passages"].as_array().into_iter().flatten().collect();
    let passages: Vec<Passage> = given
        .iter()
        .enumerate()
        .map(|(i, p)| Passage {
            // The reranker titles a passage by its file name; the fixture's
            // title stands in for it.
            path: or_empty(p, "title").replace('/', " "),
            line: i + 1,
            excerpt: or_empty(p, "excerpt"),
            source: "note".into(),
            coverage: "excerpt".into(),
            score: 0,
            semantic: None,
            lifecycle: None,
            note: None,
            current: None,
        })
        .collect();
    let calls = Cell::new(0u32);
    let started = Instant::now();
    let request_ref = request.as_str();
    let ranked = context_rank::prioritize_passages_via(passages, |items| {
        if jev.is_some() {
            calls.set(calls.get() + 1);
        }
        async move {
            let (key, endpoint) = jev?;
            Some(jev::score_with(key, endpoint, request_ref, &items, Question::Evidence).await)
        }
    })
    .await;
    let id_of = |line: usize| or_empty(given[line - 1], "id");
    let outcome = used(jev, ranked.jev.as_ref());
    result(
        &id,
        outcome,
        started,
        calls.get(),
        json!({
            "engine": ranked.engine,
            "note": ranked.note,
            "kept": ranked.passages.iter().map(|p| id_of(p.line)).collect::<Vec<_>>(),
            "decisions": ranked.decisions.iter().enumerate().map(|(i, d)| json!({
                "id": or_empty(given[i], "id"),
                "kept": d.kept,
                "score": d.score,
                "basis": d.basis,
                "reason": d.reason,
            })).collect::<Vec<_>>(),
        }),
    )
}

/// The exact rule production applies: a task's title slug
/// (`heartbeat::dedup_proposed_tasks`), a learning's normalised statement
/// (`vault_learning`'s fingerprint, without the target the fixture lacks).
fn exact_match(subject: Subject, proposal: &Record, existing: &[Record]) -> Option<usize> {
    let key = |r: &Record| match subject {
        Subject::Task => crate::vault_tasks::slugify(&r.title),
        Subject::Learning => crate::vault_learning::statement_key(&r.title),
    };
    let wanted = key(proposal);
    existing.iter().position(|e| key(e) == wanted)
}

async fn dedup_item(item: &Value, jev: Jev<'_>) -> Value {
    let id = or_empty(item, "id");
    let subject = if item["subject"] == "learning" {
        Subject::Learning
    } else {
        Subject::Task
    };
    let proposal = Record::new(
        or_empty(&item["proposal"], "title"),
        or_empty(&item["proposal"], "detail"),
    );
    let given: Vec<&Value> = item["existing"].as_array().into_iter().flatten().collect();
    let existing: Vec<Record> = given
        .iter()
        .map(|e| Record::new(or_empty(e, "title"), or_empty(e, "detail")))
        .collect();
    let of = |i: usize| json!(or_empty(given[i], "id"));
    let started = Instant::now();
    let exact = exact_match(subject, &proposal, &existing);
    let mut sent: Option<Result<(), String>> = None;
    let mut calls = 0;
    // (verdict, of, basis)
    let (verdict, target, basis) = match (jev, exact) {
        (None, Some(i)) => ("duplicate", of(i), "exact"),
        (None, None) => ("new", Value::Null, "none"),
        // Heartbeat drops exact repeats before Jev sees them.
        (Some(_), Some(i)) if subject == Subject::Task => ("duplicate", of(i), "exact"),
        (Some((key, endpoint)), _) => {
            let (checked, asked) = jev_decide::check_traced(
                key,
                endpoint,
                subject,
                std::slice::from_ref(&proposal),
                &existing,
            )
            .await;
            if asked {
                calls = 1;
                sent = Some(checked.as_ref().map(|_| ()).map_err(Clone::clone));
            }
            // Production sends nothing for an empty shortlist: its "new" is
            // the local default, not Jev's answer.
            let answered_by = if asked { "jev" } else { "none" };
            let jev_verdict = match checked {
                Ok(v) => v.first().copied().map(|v| (v, answered_by)),
                // A failed check keeps the proposal (`jev_decide::apply`).
                Err(_) => Some((Verdict::New, "fallback")),
            };
            match (jev_verdict, exact) {
                (Some((Verdict::Duplicate(i), basis)), _) => ("duplicate", of(i), basis),
                // A learning Jev kept is still collapsed by the review
                // queue's exact fingerprint.
                (_, Some(i)) => ("duplicate", of(i), "exact"),
                (Some((Verdict::Possible(i), basis)), None) => ("possible", of(i), basis),
                (Some((_, basis)), None) => ("new", Value::Null, basis),
                (None, None) => ("new", Value::Null, "none"),
            }
        }
    };
    result(
        &id,
        outcome(jev, sent.as_ref()),
        started,
        calls,
        json!({
            "verdict": verdict,
            "of": target,
            "basis": basis,
            "note": sent.and_then(Result::err),
        }),
    )
}

async fn decide(condition: Condition, item: &Value, jev: Jev<'_>) -> Result<Value, String> {
    Ok(match condition {
        Condition::Connectors => connector_item(item, jev).await,
        Condition::Passages => passage_item(item, jev).await,
        Condition::Dedup => dedup_item(item, jev).await,
    })
}

/// Write `value` to `path` whole or not at all (a temporary file renamed
/// over it), so an interrupted run never leaves half a file.
fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, format!("{text}\n"))
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| format!("write {}: {e}", path.display()))
}

fn outputs(condition: Condition, arm: &str, complete: bool, results: &[Value]) -> Value {
    json!({
        "schema": 1,
        "condition": condition.name(),
        "arm": arm,
        "complete": complete,
        "results": results,
    })
}

/// The files a study writes: `run.json` and each condition's arms.
fn study_files() -> Vec<String> {
    let mut files = vec!["run.json".to_string()];
    for c in Condition::ALL {
        files.push(format!("{}.{}.json", c.name(), c.local_arm()));
        files.push(format!("{}.jev.json", c.name()));
    }
    files
}

/// The run's record as it stands: written before the first request, and
/// before and after every item that may send one, so an interrupted billed
/// run keeps its tally (requests made, the cap, where it stopped).
///
/// `calls` counts a request from the moment it may be billed: before an
/// item is sent, `run.json` holds `calls + 1` and the item as `inFlight`;
/// after it returns, the real count and `inFlight: null`. A study killed
/// mid-request therefore leaves that request counted, and its resume (which
/// sends the item again) can't pass `--max-calls` (wave 2 follow-up).
struct Tally {
    path: PathBuf,
    record: Value,
    /// The item whose request may be in flight (`null`: none).
    in_flight: Value,
}

impl Tally {
    fn write(
        &mut self,
        state: &str,
        calls: u32,
        stopped: &Value,
        conditions: &[Value],
    ) -> Result<(), String> {
        if let Some(r) = self.record.as_object_mut() {
            r.insert("state".into(), json!(state));
            r.insert("calls".into(), json!(calls));
            r.insert("stopped".into(), stopped.clone());
            r.insert("conditions".into(), json!(conditions));
            r.insert("inFlight".into(), self.in_flight.clone());
            r.insert("updatedAt".into(), json!(now()));
            if state == "running" {
                r.remove("finishedAt");
            } else {
                r.insert("finishedAt".into(), json!(now()));
            }
        }
        write_json(&self.path, &self.record)
    }
}

/// The requests a study's Jev arm files record (every result in each, as
/// written, whether or not a resume keeps it).
fn arm_calls(out: &Path) -> u64 {
    Condition::ALL
        .iter()
        .filter_map(|c| std::fs::read(out.join(format!("{}.jev.json", c.name()))).ok())
        .filter_map(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .flat_map(|arm| arm["results"].as_array().cloned().unwrap_or_default())
        .filter_map(|r| r["calls"].as_u64())
        .sum()
}

/// What a resumed study already holds: the earlier record and each
/// condition's Jev results so far (`None`: start the arm afresh).
fn resumed(
    opts: &Options,
    fixtures: &[(Condition, Fixture)],
) -> Result<(Value, Vec<Option<(bool, Vec<Value>)>>), String> {
    let path = opts.out.join("run.json");
    let record: Value = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| {
            format!(
                "--resume needs the study's run.json in {}",
                opts.out.display()
            )
        })?;
    let kind = match &opts.target {
        Target::Loopback(_) => "loopback",
        Target::Typesafe => "typesafe",
    };
    if record["schema"] != 1 || record["endpoint"]["kind"] != kind {
        return Err(format!(
            "{} holds a study against another endpoint; resume it with the same target",
            opts.out.display()
        ));
    }
    if record["items"] != json!(opts.items) {
        return Err("--resume needs the same --items as the study it continues".into());
    }
    let mut arms = Vec::new();
    for (condition, fixture) in fixtures {
        let earlier = record["conditions"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| c["condition"] == condition.name());
        if let Some(sha) = earlier.and_then(|c| c["fixtureSha256"].as_str()) {
            if sha != fixture.sha256 {
                return Err(format!(
                    "the {} fixture changed since the study started; start a new study",
                    condition.name()
                ));
            }
        }
        let jev: Option<Value> =
            std::fs::read(opts.out.join(format!("{}.jev.json", condition.name())))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        arms.push(jev.map(|arm| {
            let results: Vec<Value> = arm["results"].as_array().cloned().unwrap_or_default();
            // Kept only as a prefix of the fixture's items, in order.
            let prefix = results
                .iter()
                .zip(&fixture.items)
                .take_while(|(r, item)| r["id"] == item["id"])
                .count();
            let complete = arm["complete"] == true && prefix == fixture.items.len();
            (complete, results[..prefix].to_vec())
        }));
    }
    Ok((record, arms))
}

/// The study: every fixture's local condition, then its Jev condition, until
/// done or `max_calls` would be passed. `key` and `endpoint` are the Jev
/// condition's; the key is never written anywhere.
///
/// A folder that already holds any study file (`run.json` or an arm, as an
/// interrupted run leaves it) is refused unless `opts.resume`; then the
/// study goes on where it stopped and `max_calls` caps its total requests.
pub(crate) async fn run(opts: &Options, key: &str, endpoint: &str) -> Result<Summary, String> {
    let held: Vec<String> = study_files()
        .into_iter()
        .filter(|f| opts.out.join(f).exists())
        .collect();
    if !held.is_empty() && !opts.resume {
        return Err(format!(
            "{} already holds a study ({}); give a new folder, or --resume to continue it",
            opts.out.display(),
            held.join(", ")
        ));
    }
    let fixtures: Vec<(Condition, Fixture)> = opts
        .conditions
        .iter()
        .map(|&c| load(&opts.fixtures, c, opts.items).map(|f| (c, f)))
        .collect::<Result<_, _>>()?;
    let (earlier, mut arms) = if opts.resume {
        let (record, arms) = resumed(opts, &fixtures)?;
        (Some(record), arms)
    } else {
        (None, vec![None; fixtures.len()])
    };
    std::fs::create_dir_all(&opts.out)
        .map_err(|e| format!("create {}: {e}", opts.out.display()))?;
    // A resume counts the larger of the tally (which includes a request
    // in flight when the study was killed) and what the arm files record (a
    // kill between an arm's write and the tally's, as the runner wrote them
    // before the in-flight record): never fewer requests than were sent.
    let tallied = earlier.as_ref().and_then(|r| r["calls"].as_u64());
    let mut calls: u32 = if earlier.is_some() {
        u32::try_from(tallied.unwrap_or(0).max(arm_calls(&opts.out))).unwrap_or(u32::MAX)
    } else {
        0
    };
    let mut record = match earlier {
        Some(mut record) => {
            if let Some(r) = record.as_object_mut() {
                r.insert("maxCalls".into(), json!(opts.max_calls));
                let mut resumes = r
                    .get("resumes")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let mut entry = json!({ "at": now(), "callsBefore": calls, "tallied": tallied });
                // The earlier run's error and in-flight item move here: the
                // record's own fields describe this run.
                if let Some(error) = r.remove("error") {
                    entry["earlierError"] = error;
                }
                if let Some(item) = r.remove("inFlight").filter(|v| !v.is_null()) {
                    entry["inFlightAtStop"] = item;
                }
                resumes.push(entry);
                r.insert("resumes".into(), json!(resumes));
            }
            record
        }
        None => json!({
            "schema": 1,
            "tool": "hollow jev-study",
            "version": env!("CARGO_PKG_VERSION"),
            "model": jev::MODEL,
            "endpoint": match &opts.target {
                Target::Loopback(url) => json!({ "kind": "loopback", "url": url }),
                Target::Typesafe => json!({ "kind": "typesafe", "url": jev::TYPESAFE_ENDPOINT }),
            },
            "fixtures": opts.fixtures.canonicalize().unwrap_or_else(|_| opts.fixtures.clone()),
            "items": opts.items,
            "maxCalls": opts.max_calls,
            "costUsd": Value::Null,
            "startedAt": now(),
        }),
    };
    if let Some(r) = record.as_object_mut() {
        r.insert("version".into(), json!(env!("CARGO_PKG_VERSION")));
    }
    let mut tally = Tally {
        path: opts.out.join("run.json"),
        record,
        in_flight: Value::Null,
    };
    let mut conditions: Vec<Value> = fixtures
        .iter()
        .zip(&arms)
        .map(|((condition, fixture), arm)| {
            json!({
                "condition": condition.name(),
                "items": fixture.items.len(),
                "fixtureSha256": fixture.sha256,
                "arms": {
                    condition.local_arm(): format!("{}.{}.json", condition.name(), condition.local_arm()),
                    "jev": format!("{}.jev.json", condition.name()),
                },
                "complete": arm.as_ref().is_some_and(|(complete, _)| *complete),
            })
        })
        .collect();
    let mut stopped = Value::Null;
    // The local condition first, for every fixture: it sends nothing.
    for (condition, fixture) in &fixtures {
        let mut results = Vec::new();
        for item in &fixture.items {
            results.push(decide(*condition, item, None).await?);
        }
        write_json(
            &opts.out.join(format!(
                "{}.{}.json",
                condition.name(),
                condition.local_arm()
            )),
            &outputs(*condition, condition.local_arm(), true, &results),
        )?;
    }
    // On disk before the first request.
    tally.write("running", calls, &stopped, &conditions)?;
    for (index, (condition, fixture)) in fixtures.iter().enumerate() {
        let (mut complete, mut results) = arms[index].take().unwrap_or((false, Vec::new()));
        let arm_file = opts.out.join(format!("{}.jev.json", condition.name()));
        if !complete && stopped.is_null() {
            for item in fixture.items.iter().skip(results.len()) {
                // Every item makes at most one request, so this stops before
                // the one that would pass the cap.
                if calls >= opts.max_calls {
                    stopped = json!({
                        "reason": format!("--max-calls {} reached", opts.max_calls),
                        "condition": condition.name(),
                        "before": item["id"],
                    });
                    break;
                }
                // Before the request: it is counted from here, so a kill
                // while it is in flight can't lose a billed request.
                tally.in_flight = json!({ "condition": condition.name(), "item": item["id"] });
                tally.write("running", calls + 1, &stopped, &conditions)?;
                let r = match decide(*condition, item, Some((key, endpoint))).await {
                    Ok(r) => r,
                    Err(e) => {
                        write_json(&arm_file, &outputs(*condition, "jev", false, &results))?;
                        if let Some(r) = tally.record.as_object_mut() {
                            r.insert("error".into(), json!(e));
                        }
                        // Whether it was sent is unknown: it stays counted.
                        tally.write("failed", calls + 1, &stopped, &conditions)?;
                        return Err(e);
                    }
                };
                let made = r["calls"].as_u64().unwrap_or(0) as u32;
                debug_assert!(made <= 1, "one request per item at most");
                calls += made;
                results.push(r);
                if made > 0 {
                    // After every request: the result, then the tally.
                    write_json(&arm_file, &outputs(*condition, "jev", false, &results))?;
                }
                tally.in_flight = Value::Null;
                tally.write("running", calls, &stopped, &conditions)?;
            }
            complete = results.len() == fixture.items.len();
        }
        write_json(&arm_file, &outputs(*condition, "jev", complete, &results))?;
        conditions[index]["complete"] = json!(complete);
        tally.write("running", calls, &stopped, &conditions)?;
    }
    let done = stopped.is_null();
    tally.write(
        if done { "complete" } else { "stopped" },
        calls,
        &stopped,
        &conditions,
    )?;
    Ok(Summary {
        calls,
        stopped: !done,
        out: opts.out.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jev::fake::{self, Reply};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn it_refuses_to_run_without_an_explicit_loopback_or_typesafe_target() {
        let refused = [
            (vec!["f", "o"], "only when told where"),
            (
                vec![
                    "f",
                    "o",
                    "--endpoint",
                    "https://api.typesafe.ai/v1/systemone",
                ],
                "loopback",
            ),
            (
                vec!["f", "o", "--endpoint", "http://localhost:9/v1/systemone"],
                "loopback",
            ),
            (
                vec!["f", "o", "--endpoint", "http://10.0.0.2:9/x"],
                "loopback",
            ),
            (
                vec!["f", "o", "--endpoint", "http://u:p@127.0.0.1:9/x"],
                "loopback",
            ),
            (
                vec!["f", "o", "--endpoint", "http://127.0.0.1:9/x", "--typesafe"],
                "not both",
            ),
            (
                vec!["f", "o", "--typesafe", "--max-calls", "0"],
                "--max-calls",
            ),
            (
                vec!["f", "o", "--typesafe", "--max-calls", "lots"],
                "--max-calls",
            ),
            (
                vec!["f", "--typesafe"],
                "fixture folder and an output folder",
            ),
            (
                vec!["f", "o", "--typesafe", "--condition", "model-routing"],
                "--condition",
            ),
            (
                vec!["f", "o", "--typesafe", "--retries", "3"],
                "unknown flag",
            ),
        ];
        for (list, why) in refused {
            let err = parse(&args(&list)).unwrap_err();
            assert!(err.contains(why), "{list:?}: {err}");
        }
        assert_eq!(
            main(args(&["f", "o"])),
            2,
            "a refusal exits 2 before anything runs"
        );
        let loopback = parse(&args(&[
            "f",
            "o",
            "--endpoint",
            "http://127.1:4/v1/systemone",
        ]))
        .unwrap();
        assert_eq!(
            loopback.target,
            Target::Loopback("http://127.0.0.1:4/v1/systemone".into())
        );
        assert_eq!(loopback.max_calls, DEFAULT_MAX_CALLS);
        assert_eq!(loopback.conditions, Condition::ALL.to_vec());
        let typesafe = parse(&args(&[
            "f",
            "o",
            "--typesafe",
            "--max-calls",
            "7",
            "--items",
            "2",
            "--condition",
            "proposal-dedup",
            "--condition",
            "connector-relevance",
        ]))
        .unwrap();
        assert_eq!(typesafe.target, Target::Typesafe);
        assert_eq!((typesafe.max_calls, typesafe.items), (7, Some(2)));
        assert_eq!(
            typesafe.conditions,
            vec![Condition::Connectors, Condition::Dedup]
        );
    }

    /// Answers every question Jev could be asked: a score question gets
    /// 3 for even-numbered candidates and 0 for odd ones; a choice question
    /// puts all its weight on its first option other than `none` (for a
    /// duplicate check, the best shortlisted record).
    fn study_answers(body: &Value) -> Value {
        let mut answers = serde_json::Map::new();
        for (name, q) in body["questions"].as_object().into_iter().flatten() {
            let answer = if q["type"] == "score" {
                let i: usize = name
                    .rsplit('_')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                json!({ "type": "score", "score": if i.is_multiple_of(2) { 3.0 } else { 0.0 } })
            } else {
                let keys: Vec<&String> = q["criteria"]
                    .as_object()
                    .map(|c| c.keys().collect())
                    .unwrap_or_default();
                let last = keys
                    .iter()
                    .find(|k| k.as_str() != "none")
                    .map_or("none", |k| k.as_str());
                let probabilities: serde_json::Map<String, Value> = keys
                    .iter()
                    .map(|k| {
                        (
                            k.to_string(),
                            json!(if k.as_str() == last { 1.0 } else { 0.0 }),
                        )
                    })
                    .collect();
                json!({ "type": "choice", "choice": last, "confidence": 1.0, "probabilities": probabilities })
            };
            answers.insert(name.clone(), answer);
        }
        json!({ "answers": answers })
    }

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/agent-eval/jev-fixtures")
    }

    fn read(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn opts(out: &Path, url: &str, items: Option<usize>) -> Options {
        Options {
            fixtures: fixtures_dir(),
            out: out.to_path_buf(),
            target: Target::Loopback(url.into()),
            max_calls: DEFAULT_MAX_CALLS,
            items,
            conditions: Condition::ALL.to_vec(),
            resume: false,
        }
    }

    /// The study folder [`tally_then_answer`] watches, and the `calls`
    /// each request found in its `run.json` (`None`: no tally on disk).
    static WATCHED: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
    static TALLIES: std::sync::Mutex<Vec<Option<u64>>> = std::sync::Mutex::new(Vec::new());

    fn tally_then_answer(body: &Value) -> Value {
        let out = WATCHED.lock().unwrap().clone();
        let calls = out
            .and_then(|out| std::fs::read(out.join("run.json")).ok())
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|run| run["calls"].as_u64());
        TALLIES.lock().unwrap().push(calls);
        study_answers(body)
    }

    /// E3 follow-up (red first): an interrupted billed run keeps its tally.
    /// `run.json` (requests made, the cap, where it stopped) is on disk
    /// before the first request and rewritten around every one. Since the
    /// wave 2 follow-up a request is counted before it is sent, so request
    /// k finds k counted: the k - 1 before it and itself, in flight.
    #[tokio::test]
    async fn the_tally_is_on_disk_after_every_request() {
        let server = fake::serve(Reply::With(tally_then_answer));
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("study");
        *WATCHED.lock().unwrap() = Some(out.clone());
        let mut o = opts(&out, &server.url, Some(4));
        o.conditions = vec![Condition::Connectors, Condition::Dedup];
        let summary = run(&o, LOOPBACK_KEY, &server.url).await.unwrap();
        let tallies = TALLIES.lock().unwrap().clone();
        assert!(summary.calls >= 2, "the run made requests");
        assert_eq!(tallies.len() as u32, summary.calls);
        assert_eq!(
            tallies,
            (1..=summary.calls)
                .map(|k| Some(u64::from(k)))
                .collect::<Vec<_>>(),
            "the tally each request found on disk"
        );
        let record = read(&out.join("run.json"));
        assert_eq!(record["calls"], summary.calls);
        assert_eq!(record["maxCalls"], DEFAULT_MAX_CALLS);
        assert_eq!(record["stopped"], Value::Null);
    }

    /// E3 follow-up (red first): a folder an interrupted run left (arm files,
    /// no `run.json`) is refused, not silently overwritten, and nothing is
    /// sent.
    #[tokio::test]
    async fn a_folder_holding_an_arm_file_is_refused() {
        let server = fake::serve(Reply::With(study_answers));
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("study");
        std::fs::create_dir_all(&out).unwrap();
        let arm = out.join("connector-relevance.jev.json");
        std::fs::write(&arm, "{\"partial\":true}").unwrap();
        let refused = run(&opts(&out, &server.url, Some(1)), LOOPBACK_KEY, &server.url).await;
        let error = refused
            .err()
            .expect("an interrupted run's folder is refused");
        assert!(error.contains("already holds"), "{error}");
        assert_eq!(std::fs::read_to_string(&arm).unwrap(), "{\"partial\":true}");
        assert!(server.seen.lock().unwrap().is_empty(), "nothing was sent");
    }

    /// E3 follow-up: `--resume` continues a stopped (or interrupted) study
    /// in its own folder: finished arms are kept, the stopped arm goes on
    /// from its first unanswered item, nothing answered is asked again, and
    /// `--max-calls` caps the study's total. It needs the study's run.json
    /// and the same target and items.
    #[tokio::test]
    async fn resume_continues_a_stopped_study_without_asking_again() {
        let parsed = parse(&args(&[
            "f",
            "o",
            "--endpoint",
            "http://127.0.0.1:9/x",
            "--resume",
        ]))
        .unwrap();
        assert!(parsed.resume);
        let server = fake::serve(Reply::With(study_answers));
        let dir = tempfile::tempdir().unwrap();
        // The same study uninterrupted, for comparison.
        let whole = dir.path().join("whole");
        let full = run(
            &opts(&whole, &server.url, Some(4)),
            LOOPBACK_KEY,
            &server.url,
        )
        .await
        .unwrap();
        let before = server.seen.lock().unwrap().len();
        let out = dir.path().join("study");
        let mut capped = opts(&out, &server.url, Some(4));
        capped.max_calls = 3;
        let first = run(&capped, LOOPBACK_KEY, &server.url).await.unwrap();
        assert!(first.stopped);
        assert_eq!(read(&out.join("run.json"))["state"], "stopped");
        // Without --resume the folder is refused.
        let again = run(&opts(&out, &server.url, Some(4)), LOOPBACK_KEY, &server.url).await;
        assert!(again.err().unwrap().contains("--resume"));
        let mut resume = opts(&out, &server.url, Some(4));
        resume.resume = true;
        let finished = run(&resume, LOOPBACK_KEY, &server.url).await.unwrap();
        assert!(!finished.stopped);
        assert_eq!(finished.calls, full.calls, "the study's total");
        assert_eq!(
            server.seen.lock().unwrap().len() - before,
            full.calls as usize,
            "nothing answered was asked again"
        );
        let record = read(&out.join("run.json"));
        assert_eq!(record["state"], "complete");
        assert_eq!(record["resumes"].as_array().unwrap().len(), 1);
        assert_eq!(record["resumes"][0]["callsBefore"], 3);
        for c in Condition::ALL {
            let a = read(&out.join(format!("{}.jev.json", c.name())));
            let b = read(&whole.join(format!("{}.jev.json", c.name())));
            assert_eq!(a["complete"], true, "{}", c.name());
            let ids = |v: &Value| {
                v["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["id"].clone())
                    .collect::<Vec<_>>()
            };
            assert_eq!(ids(&a), ids(&b), "{}", c.name());
        }
        // A folder with no run.json can't be resumed; another target is refused.
        let empty = dir.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let mut nothing = opts(&empty, &server.url, Some(1));
        nothing.resume = true;
        assert!(run(&nothing, LOOPBACK_KEY, &server.url)
            .await
            .err()
            .unwrap()
            .contains("run.json"));
        let mut other = resume.clone();
        other.items = Some(2);
        assert!(run(&other, LOOPBACK_KEY, &server.url)
            .await
            .err()
            .unwrap()
            .contains("--items"));
    }

    /// [`held_answer`]'s request count, the request it holds (1-based; 0:
    /// none) and the flag that lets it answer.
    static HELD_SEEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static HOLD_AT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static RELEASE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// [`study_answers`], but request [`HOLD_AT`] gets no answer until
    /// [`RELEASE`]: the study is killed while it waits.
    fn held_answer(body: &Value) -> Value {
        use std::sync::atomic::Ordering::SeqCst;
        let n = HELD_SEEN.fetch_add(1, SeqCst) + 1;
        if n == HOLD_AT.load(SeqCst) {
            while !RELEASE.load(SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        study_answers(body)
    }

    /// Wave 2 follow-up (red first): a study killed while a request is in
    /// flight. That request may be billed, but `run.json` counted a request
    /// only after it returned, so the resumed study sent it again uncounted
    /// and passed `--max-calls`. The in-flight request is on disk before it
    /// is sent; a resume counts it and the cap holds across resumes.
    #[tokio::test]
    async fn a_request_in_flight_when_the_study_is_killed_is_counted() {
        use std::sync::atomic::Ordering::SeqCst;
        let server = fake::serve(Reply::With(held_answer));
        HOLD_AT.store(2, SeqCst);
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("study");
        let mut capped = opts(&out, &server.url, Some(4));
        capped.max_calls = 3;
        {
            let study = run(&capped, LOOPBACK_KEY, &server.url);
            tokio::pin!(study);
            let in_flight = async {
                while server.seen.lock().unwrap().len() < 2 {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            };
            tokio::select! {
                done = &mut study => panic!("the study finished before the kill: {:?}", done.map(|s| s.calls)),
                _ = in_flight => {}
            }
            // Dropped here, mid-request: nothing after the send runs.
        }
        RELEASE.store(true, SeqCst);
        let killed = read(&out.join("run.json"));
        assert_eq!(
            killed["calls"], 2,
            "the request in flight at the kill is counted: {killed}"
        );
        assert!(killed["inFlight"].is_object(), "{killed}");
        let mut resume = capped.clone();
        resume.resume = true;
        let resumed = run(&resume, LOOPBACK_KEY, &server.url).await.unwrap();
        let sent = server.seen.lock().unwrap().len();
        assert!(
            sent <= 3,
            "--max-calls 3, but the study sent {sent} requests across the kill and the resume"
        );
        assert_eq!(
            resumed.calls as usize, sent,
            "every request sent is counted"
        );
        let record = read(&out.join("run.json"));
        assert_eq!(record["calls"], sent);
        assert_eq!(record["inFlight"], Value::Null, "{record}");
        assert_eq!(record["state"], "stopped");
    }

    /// Wave 2 follow-up (red first): a study whose `run.json` fell behind
    /// its arm files (killed after writing the arm, before the tally, as the
    /// runner wrote them until 2026-10-07) resumes from the larger count,
    /// so the cap still holds; a resumed study that completes clears the
    /// earlier run's error.
    #[tokio::test]
    async fn a_resume_takes_the_larger_count_and_clears_a_stale_error() {
        let server = fake::serve(Reply::With(study_answers));
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("study");
        let mut capped = opts(&out, &server.url, Some(4));
        capped.max_calls = 2;
        let first = run(&capped, LOOPBACK_KEY, &server.url).await.unwrap();
        assert!(first.stopped);
        assert_eq!(first.calls, 2);
        // The arm holds both results; the tally says one, and the run failed.
        let mut record = read(&out.join("run.json"));
        record["calls"] = json!(1);
        record["state"] = json!("failed");
        record["error"] = json!("killed between the arm and the tally");
        write_json(&out.join("run.json"), &record).unwrap();
        let before = server.seen.lock().unwrap().len();
        let mut resume = capped.clone();
        resume.resume = true;
        let again = run(&resume, LOOPBACK_KEY, &server.url).await.unwrap();
        assert_eq!(
            server.seen.lock().unwrap().len(),
            before,
            "--max-calls 2 was already spent: nothing more is sent"
        );
        assert_eq!(again.calls, 2);
        assert!(again.stopped);
        // Finished with a higher cap: the stale error goes.
        resume.max_calls = DEFAULT_MAX_CALLS;
        let finished = run(&resume, LOOPBACK_KEY, &server.url).await.unwrap();
        assert!(!finished.stopped);
        let record = read(&out.join("run.json"));
        assert_eq!(record["state"], "complete");
        assert!(
            record.get("error").is_none(),
            "a stale error stayed: {record}"
        );
        assert_eq!(
            record["calls"],
            json!(server.seen.lock().unwrap().len()),
            "the study's total is every request it sent"
        );
    }

    /// E3 follow-up (red first): a duplicate check production never sent
    /// (an empty shortlist) is not labelled as Jev's decision.
    #[tokio::test]
    async fn a_dedup_jev_never_saw_is_not_labelled_jev() {
        let server = fake::serve(Reply::With(study_answers));
        let item = json!({
            "id": "d-empty", "subject": "task",
            "proposal": { "title": "Book the venue for the launch", "detail": "" },
            "existing": []
        });
        let r = dedup_item(&item, Some((LOOPBACK_KEY, &server.url))).await;
        assert_eq!(
            (r["calls"].clone(), r["outcome"].clone()),
            (json!(0), json!("not-sent"))
        );
        assert_eq!(r["verdict"], "new");
        assert_ne!(r["basis"], "jev", "{r}");
        assert!(server.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn every_use_runs_through_production_code_with_jev_off_and_on() {
        let server = fake::serve(Reply::With(study_answers));
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("study");
        let summary = run(&opts(&out, &server.url, None), LOOPBACK_KEY, &server.url)
            .await
            .unwrap();
        assert!(!summary.stopped);
        let record = read(&out.join("run.json"));
        assert_eq!(record["calls"], summary.calls);
        assert_eq!(record["costUsd"], Value::Null);
        assert_eq!(record["endpoint"]["kind"], "loopback");
        assert!(
            !record.to_string().contains(LOOPBACK_KEY),
            "the key is never written"
        );
        let sent: usize = server.seen.lock().unwrap().len();
        assert_eq!(
            sent as u64,
            record["calls"].as_u64().unwrap(),
            "every request is counted"
        );
        for condition in Condition::ALL {
            let fixture = read(&fixtures_dir().join(format!("{}.json", condition.name())));
            let items = fixture["items"].as_array().unwrap();
            let local = read(&out.join(format!(
                "{}.{}.json",
                condition.name(),
                condition.local_arm()
            )));
            let jev = read(&out.join(format!("{}.jev.json", condition.name())));
            for (arm, file) in [("local", &local), ("jev", &jev)] {
                assert_eq!(file["complete"], true);
                let results = file["results"].as_array().unwrap();
                assert_eq!(
                    results.len(),
                    items.len(),
                    "{} {arm}: one result per item",
                    condition.name()
                );
                for r in results {
                    assert!(r["latencyMs"].as_f64().is_some());
                    assert!(r["calls"].as_u64().unwrap() <= 1);
                    if r["calls"] == 0 {
                        assert_eq!(r["costUsd"], 0, "no request is a known 0");
                    } else {
                        assert_eq!(r["costUsd"], Value::Null, "TypeSafe's cost is unknown");
                    }
                }
            }
            assert!(local["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["outcome"] == "off" && r["calls"] == 0));
            assert!(jev["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["outcome"] == "ok" || r["outcome"] == "not-sent"));
        }
        // Connectors: one decision per candidate, a named one is never sent,
        // and past 24 is local (production's F5 contract), in both arms.
        let jev = read(&out.join("connector-relevance.jev.json"));
        let fixture = read(&fixtures_dir().join("connector-relevance.json"));
        for (r, item) in jev["results"]
            .as_array()
            .unwrap()
            .iter()
            .zip(fixture["items"].as_array().unwrap())
        {
            assert_eq!(
                r["decisions"].as_array().unwrap().len(),
                item["candidates"].as_array().unwrap().len()
            );
            for named in item["named"].as_array().into_iter().flatten() {
                let d = r["decisions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|d| d["id"] == *named)
                    .unwrap();
                assert_eq!(
                    (d["attach"].clone(), d["basis"].clone()),
                    (json!(true), json!("named")),
                    "{}",
                    item["id"]
                );
            }
        }
        assert!(
            server
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|&n| n <= jev::MAX_CANDIDATES),
            "never more than 24 per request"
        );
        let big = jev["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["decisions"].as_array().unwrap().len() > 25)
            .expect("a fixture item past 25 candidates");
        assert!(big["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["basis"] == "beyond-limit"));
        // Passages: the fake keeps the even-numbered, so the Jev arm drops some.
        let kept = read(&out.join("passage-rerank.jev.json"));
        assert!(kept["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["engine"] == "jev" && r["note"].is_string()));
        let local = read(&out.join("passage-rerank.local.json"));
        for (l, item) in local["results"].as_array().unwrap().iter().zip(
            read(&fixtures_dir().join("passage-rerank.json"))["items"]
                .as_array()
                .unwrap(),
        ) {
            assert_eq!(
                l["kept"].as_array().unwrap().len(),
                item["passages"].as_array().unwrap().len(),
                "local keeps all, in order"
            );
        }
        // Dedup: the fake picks the best shortlisted record (≥ 0.85: drop).
        let dedup = read(&out.join("proposal-dedup.jev.json"));
        assert!(dedup["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["verdict"] == "duplicate" && r["basis"] == "jev"));
        let exact = read(&out.join("proposal-dedup.exact.json"));
        assert!(exact["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["basis"] == "exact" || r["verdict"] == "new"));
        // A folder that already holds a study is refused, not overwritten.
        let again = run(&opts(&out, &server.url, None), LOOPBACK_KEY, &server.url).await;
        assert!(again.err().unwrap().contains("already holds a study"));
    }

    #[tokio::test]
    async fn max_calls_stops_before_the_request_that_would_pass_it() {
        let server = fake::serve(Reply::With(study_answers));
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("capped");
        let mut o = opts(&out, &server.url, None);
        o.max_calls = 2;
        let summary = run(&o, LOOPBACK_KEY, &server.url).await.unwrap();
        assert!(summary.stopped);
        assert_eq!(summary.calls, 2);
        assert_eq!(
            server.seen.lock().unwrap().len(),
            2,
            "exactly the cap, never one more"
        );
        let record = read(&out.join("run.json"));
        assert_eq!(record["stopped"]["condition"], "connector-relevance");
        assert!(record["stopped"]["reason"]
            .as_str()
            .unwrap()
            .contains("--max-calls 2"));
        let conditions = record["conditions"].as_array().unwrap();
        assert!(conditions.iter().all(|c| c["complete"] == false));
        // The local condition ran in full for every use.
        for condition in Condition::ALL {
            let local = read(&out.join(format!(
                "{}.{}.json",
                condition.name(),
                condition.local_arm()
            )));
            assert_eq!(local["complete"], true);
            assert_eq!(
                read(&out.join(format!("{}.jev.json", condition.name())))["complete"],
                false
            );
        }
    }

    #[tokio::test]
    async fn a_malformed_answer_keeps_the_local_decision_and_says_so() {
        let server = fake::serve(Reply::Raw("{\"answers\": [not json".into()));
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("malformed");
        run(&opts(&out, &server.url, Some(3)), LOOPBACK_KEY, &server.url)
            .await
            .unwrap();
        for condition in Condition::ALL {
            let local = read(&out.join(format!(
                "{}.{}.json",
                condition.name(),
                condition.local_arm()
            )));
            let jev = read(&out.join(format!("{}.jev.json", condition.name())));
            for (l, j) in local["results"]
                .as_array()
                .unwrap()
                .iter()
                .zip(jev["results"].as_array().unwrap())
            {
                assert!(
                    j["outcome"] == "malformed" || j["outcome"] == "not-sent",
                    "{}: {}",
                    condition.name(),
                    j["outcome"]
                );
                // The fallback is the local decision, so the answer is there.
                let attach = |r: &Value| {
                    r["decisions"]
                        .as_array()
                        .map(|d| d.iter().map(|x| x["attach"].clone()).collect::<Vec<_>>())
                };
                assert_eq!(attach(l), attach(j), "{}", condition.name());
                for key in ["kept", "verdict", "of"] {
                    assert_eq!(l[key], j[key], "{} {key}", condition.name());
                }
            }
        }
    }

    /// The study folders `scripts/agent-eval-jev.test.mjs` scores
    /// (`scripts/agent-eval/fixtures/jev-study/<mode>/`) are what this
    /// runner writes against the loopback fake in score, timeout and
    /// malformed mode, apart from times, the fake's port and the version.
    /// `HOLLOW_BLESS_JEV_STUDY=1 cargo test --lib jev_study` rewrites them.
    #[tokio::test]
    async fn the_recorded_study_folders_are_what_the_runner_writes() {
        let recorded =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/agent-eval/fixtures/jev-study");
        let bless = std::env::var_os("HOLLOW_BLESS_JEV_STUDY").is_some();
        let modes: [(&str, Reply, usize); 3] = [
            ("score", Reply::With(study_answers), 4),
            ("timeout", Reply::Silent, 1),
            ("malformed", Reply::Raw("{\"answers\": [not json".into()), 4),
        ];
        for (mode, reply, items) in modes {
            let server = fake::serve(reply);
            let dir = tempfile::tempdir().unwrap();
            let out = dir.path().join(mode);
            let started = Instant::now();
            let summary = run(
                &opts(&out, &server.url, Some(items)),
                LOOPBACK_KEY,
                &server.url,
            )
            .await
            .unwrap();
            assert!(!summary.stopped, "{mode}");
            if mode == "timeout" {
                assert_eq!(summary.calls, 3, "one request per use, never retried");
                assert!(
                    started.elapsed() < std::time::Duration::from_secs(24),
                    "4 s each"
                );
                for c in Condition::ALL {
                    let r = &read(&out.join(format!("{}.jev.json", c.name())))["results"][0];
                    assert_eq!(r["outcome"], "timeout", "{}", c.name());
                    assert!(r["latencyMs"].as_f64().unwrap() >= 3_900.0);
                    assert!(
                        r.get(match c {
                            Condition::Connectors => "decisions",
                            Condition::Passages => "kept",
                            Condition::Dedup => "verdict",
                        })
                        .is_some(),
                        "the fallback decision is recorded"
                    );
                }
            }
            let mut names: Vec<String> = std::fs::read_dir(&out)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            for name in names {
                let mut got = read(&out.join(&name));
                // Machine- and moment-specific values.
                if name == "run.json" {
                    got["endpoint"]["url"] = json!("http://127.0.0.1:0/v1/systemone");
                    got["fixtures"] = json!("scripts/agent-eval/jev-fixtures");
                    got["version"] = json!("test");
                }
                let target = recorded.join(mode).join(&name);
                if bless {
                    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                    write_json(&target, &got).unwrap();
                    continue;
                }
                let strip = |v: &mut Value| {
                    for key in ["startedAt", "finishedAt", "updatedAt"] {
                        v.as_object_mut().map(|o| o.remove(key));
                    }
                    for r in v["results"].as_array_mut().into_iter().flatten() {
                        r.as_object_mut().map(|o| o.remove("latencyMs"));
                    }
                };
                let mut want =
                    serde_json::from_slice::<Value>(&std::fs::read(&target).unwrap_or_else(|_| {
                        panic!(
                            "{} is missing; bless with HOLLOW_BLESS_JEV_STUDY=1",
                            target.display()
                        )
                    }))
                    .unwrap();
                strip(&mut got);
                strip(&mut want);
                assert_eq!(got, want, "{mode}/{name} differs from what the runner writes; if the change is intended, bless with HOLLOW_BLESS_JEV_STUDY=1");
            }
        }
    }

    #[test]
    fn fixtures_production_would_refuse_are_refused_before_any_request() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, v: Value| {
            std::fs::write(dir.path().join(format!("{name}.json")), v.to_string()).unwrap()
        };
        write(
            "passage-rerank",
            json!({"schema": 1, "condition": "passage-rerank", "items": [{"id": "big", "request": "q",
            "passages": (0..25).map(|i| json!({"id": format!("p{i}"), "title": "t", "excerpt": "e"})).collect::<Vec<_>>()}]}),
        );
        assert!(load(dir.path(), Condition::Passages, None)
            .err()
            .unwrap()
            .contains("at most 24"));
        // The retired whiteboard-layout use is no longer a condition.
        assert!(Condition::parse("whiteboard-layout").is_none());
        write(
            "proposal-dedup",
            json!({"schema": 1, "condition": "connector-relevance", "items": []}),
        );
        assert!(load(dir.path(), Condition::Dedup, None)
            .err()
            .unwrap()
            .contains("not a schema 1"));
        assert!(
            load(dir.path(), Condition::Connectors, None).is_err(),
            "a missing fixture"
        );
    }

    #[test]
    fn the_exact_rules_are_productions() {
        let existing = [
            Record::new("Chase the vendor for gateway firmware 2.4", ""),
            Record::new("Kestrel reports  go out every   Friday.", ""),
        ];
        assert_eq!(
            exact_match(
                Subject::Task,
                &Record::new("chase the vendor for gateway-firmware 2.4!", ""),
                &existing
            ),
            Some(0)
        );
        assert_eq!(
            exact_match(
                Subject::Learning,
                &Record::new("kestrel reports go out every friday.", ""),
                &existing
            ),
            Some(1)
        );
        assert_eq!(
            exact_match(
                Subject::Learning,
                &Record::new("Kestrel reports go out every Friday", ""),
                &existing
            ),
            None,
            "punctuation counts for a learning"
        );
    }
}
