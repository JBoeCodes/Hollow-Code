//! `hollow_code_context`'s nine tools: `map`, `symbols`, `callers`,
//! `callees`, `impact` and `read` (CC2), `slice` (CC4,
//! `code_context_slice.rs`), and `changes` and `affected_tests` (CC5,
//! `code_context_changes.rs`) over the checkout's code index (`code_index`). One surface, shared by every transport: the MCP stdio
//! server and the shared endpoint (`code_context_mcp`) and the CLI twin
//! (`code_context_cli`) all call [`specs`] and [`call`], so each returns the
//! same bytes.
//!
//! The contract (CC1 design 2.2): the name scan is the floor (every hit is
//! listed, resolved or not); resolution only annotates (`resolved`, `how`,
//! `totals.resolvesElsewhere`); every hit names its file's 12-hex hash and
//! the files a result returns are re-hashed, a changed one flagged
//! `stale:true` (and the envelope's `complete` false); every result carries
//! the repo's head, fingerprint, `indexedAt` and `dirtyFiles`. Results are
//! held to OB1's byte budget (`mcp_envelope`, default 16 KB here) and paged
//! by its cursor. Read only: nothing in the repo runs.

use crate::code_index::{self, store::EdgeRow, store::Index, store::NodeRow};
use crate::mcp_envelope::{self as env, Request, Server};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The checkout root the server reads (`server_entry` writes it).
pub const ENV_ROOT: &str = "HOLLOW_CODE_ROOT";
/// Where the index files live (tests; the OS cache folder otherwise).
pub const ENV_INDEX_DIR: &str = "HOLLOW_CODE_INDEX_DIR";
/// The vault root, so the server refuses a checkout that holds it.
pub const ENV_VAULT: &str = "HOLLOW_CODE_VAULT";

/// This server's default text budget (CC1 design 2.2 rule 4).
pub(crate) const DEFAULT_MAX_BYTES: usize = 16 * 1024;
/// `impact` refuses past this many functions.
pub(crate) const IMPACT_MAX_NODES: usize = 200;
const IMPACT_MAX_DEPTH: u64 = 4;
const SYMBOLS_MAX: u64 = 100;
/// `definitions` lists at most this many (sorted by path and line); the
/// rest are counted in `definitionsTotal`, so a page holds its budget.
pub(crate) const DEFINITIONS_SHOWN: usize = 20;
/// The argument the cursor's query hash folds the index fingerprint in as
/// (removed again before the tool sees its arguments).
const ARG_INDEX: &str = "\u{0}indexFingerprint";

/// The checkout one server reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Target {
    pub root: PathBuf,
    pub index_dir: Option<PathBuf>,
    /// The vault root (`HOLLOW_CODE_VAULT`): a root that holds it, or sits
    /// inside it, is refused.
    pub vault: Option<PathBuf>,
}

impl Target {
    pub(crate) fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        let path = |k: &str| {
            get(k)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        };
        Target {
            root: path(ENV_ROOT).unwrap_or_default(),
            index_dir: path(ENV_INDEX_DIR),
            vault: path(ENV_VAULT),
        }
    }
}

/// Why `root` may not be served: it is the home folder, or it holds or
/// sits inside the vault (the vault has `hollow_vault`). `None` when it may.
pub(crate) fn root_refusal(root: &Path, vault: Option<&Path>) -> Option<String> {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let root = canon(root);
    if dirs::home_dir().is_some_and(|home| canon(&home) == root) {
        return Some(format!(
            "REFUSED: `{}` is the home folder, not a project checkout; hollow_code_context reads a project's own git checkout only. Nothing was read.",
            root.display()
        ));
    }
    if let Some(vault) = vault.filter(|v| !v.as_os_str().is_empty()) {
        let vault = canon(vault);
        if vault.starts_with(&root) || root.starts_with(&vault) {
            return Some(format!(
                "REFUSED: `{}` holds or sits inside the vault, which hollow_code_context never reads (use hollow_vault). Nothing was read.",
                root.display()
            ));
        }
    }
    None
}

// ── Specs ───────────────────────────────────────────────────────────────

fn s(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn spec(
    name: &str,
    description: &str,
    props: Vec<(&str, Value)>,
    required: &[&str],
    enveloped: bool,
) -> Value {
    let props: Map<String, Value> = props.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    let mut tool = json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": props, "required": required, "additionalProperties": false },
    });
    if enveloped {
        env::add_schema(&mut tool);
        tool["inputSchema"]["properties"][env::ARG_MAX_BYTES]["description"] = json!(format!(
            "Text budget in bytes (default {DEFAULT_MAX_BYTES})."
        ));
    }
    tool
}

/// The nine tools' schemas, in a fixed order.
pub(crate) fn specs() -> Vec<Value> {
    let name = || s("Symbol name as written (fn, method, type, const).");
    let qualifier =
        || s("Impl type, class or module part that narrows it (vault_mcp, AttachedCaps).");
    let languages = || json!({ "type": "array", "items": { "type": "string", "enum": ["rust", "ts", "python"] }, "description": "Language families (default: the definitions' own)." });
    let tests = || json!({ "type": "boolean", "description": "Include test code (default true; flagged test:true)." });
    let base = || s("Git rev to diff from (default: merge-base with origin/HEAD).");
    let paths = || json!({ "type": "array", "items": { "type": "string" }, "description": "Only these repo-relative files or folders." });
    vec![
        spec("map", "Outline a folder (files and definition counts) or a file (its definitions). Start here.",
            vec![("path", s("Repo-relative file or folder (default: the root)."))], &[], true),
        spec("symbols", "Find definitions by name words: \"skill valid\" finds valid_skill_name.",
            vec![("query", s("Words or a name.")), ("kind", s("fn, method, struct, class, enum, trait, iface, type, const, static, mod, macro or var.")),
                 ("languages", languages()), ("limit", json!({ "type": "integer", "minimum": 1, "maximum": SYMBOLS_MAX, "description": "At most this many (default 20)." }))],
            &["query"], true),
        spec("callers", "Who uses a symbol. Every name-scan hit is listed; resolved:true marks those a rule ties to it.",
            vec![("name", name()), ("qualifier", qualifier()), ("languages", languages()),
                 ("precision", json!({ "type": "string", "enum": ["all", "resolved"], "description": "all (default) or only resolved hits." })),
                 ("includeTests", tests())],
            &["name"], true),
        spec("callees", "What a function calls, each resolved to its definition where a rule allows.",
            vec![("name", name()), ("qualifier", qualifier()), ("languages", languages())], &["name"], true),
        spec("impact", "Functions that reach a symbol through resolved calls, by depth; unresolved hits counted.",
            vec![("name", name()), ("qualifier", qualifier()), ("languages", languages()),
                 ("depth", json!({ "type": "integer", "minimum": 1, "maximum": IMPACT_MAX_DEPTH, "description": "Levels of callers (default 2)." })),
                 ("includeTests", tests())],
            &["name"], true),
        spec("read", "Numbered lines of an indexed file (600 lines, 16384 bytes at most; a longer single line is cut and flagged), or one symbol's definition.",
            vec![("path", s("Repo-relative path.")), ("fromLine", json!({ "type": "integer", "minimum": 1 })),
                 ("toLine", json!({ "type": "integer", "minimum": 1 })), ("symbol", s("Read this definition's lines instead."))],
            &["path"], false),
        spec("slice", "Cards for a symbol (or task) and its callers and callees. Pass handle back for only changed cards or notModified.",
            vec![("symbol", s("Symbol name (or give task).")), ("qualifier", qualifier()), ("languages", languages()),
                 ("task", s("Free text; picks the 3 best-matching definitions.")),
                 ("depth", json!({ "type": "integer", "minimum": 1, "maximum": slice::SLICE_MAX_DEPTH, "description": "Hops of callers and callees (default 1)." })),
                 ("handle", s("An earlier slice's handle."))],
            &[], true),
        spec("changes", "Files and definitions changed since base, work tree included; unmapped files with a reason.",
            vec![("base", base()), ("paths", paths())], &[], true),
        spec("affected_tests", "Tests covering a change: cargo filters and test files with why and confidence, cannotMap, missRate. Not complete: run the full gate too.",
            vec![("base", base()), ("paths", paths()),
                 ("symbols", json!({ "type": "array", "items": { "type": "string" }, "description": "Names (or Type::name) instead of a diff." })),
                 ("depth", json!({ "type": "integer", "minimum": 1, "maximum": changes::AFFECTED_MAX_DEPTH, "description": "Levels of non-test callers (default 2)." }))],
            &[], true),
    ]
}

// ── Dispatch ────────────────────────────────────────────────────────────

/// One tool call: the MCP `tools/call` result (`content`, `isError`).
pub(crate) fn call(name: &str, args: &Value, target: &Target) -> Value {
    match prepare(name, args, target) {
        Ok((req, args, index)) => match answer(name, &args, &req, &index, target) {
            Ok(content) => json!({ "content": content, "isError": false }),
            Err(e) => refusal(&format!("{name}: {e}")),
        },
        Err(e) => refusal(&e),
    }
}

fn refusal(text: &str) -> Value {
    json!({ "content": env::refusal_content(Server::Code, text), "isError": true })
}

type Prepared = (Option<Request>, Map<String, Value>, Index);

fn prepare(name: &str, args: &Value, target: &Target) -> Result<Prepared, String> {
    let specs = specs();
    let Some(spec) = specs.iter().find(|s| s["name"] == name) else {
        return Err(format!("unknown tool: {name}; tools/list names map, symbols, callers, callees, impact, read, slice, changes and affected_tests."));
    };
    let mut args = if args.is_null() {
        json!({})
    } else {
        args.clone()
    };
    let enveloped = spec.pointer("/inputSchema/properties/cursor").is_some();
    if enveloped {
        if let Some(map) = args.as_object_mut() {
            if map.get(env::ARG_MAX_BYTES).is_none_or(Value::is_null) {
                map.insert(env::ARG_MAX_BYTES.into(), json!(DEFAULT_MAX_BYTES));
            }
        }
    }
    // Check the arguments before any indexing (the cursor waits for the
    // index's fingerprint, below).
    let mut probe = args.clone();
    if enveloped {
        if let Some(map) = probe.as_object_mut() {
            map.remove(env::ARG_CURSOR);
        }
        Request::take(name, &mut probe).map_err(|e| e.replace("default 24576", "default 16384"))?;
    }
    let map = check_args(name, spec, &probe)?;
    if target.root.as_os_str().is_empty() {
        return Err(format!("{name}: hollow_code_context is unavailable here: no checkout root ({ENV_ROOT}). Nothing was read."));
    }
    if let Some(why) = root_refusal(&target.root, target.vault.as_deref()) {
        return Err(format!("{name}: {why}"));
    }
    // CC3: a base shared by the repository's worktrees plus this one's
    // overlay, else the full index.
    let index = code_index::open(&target.root, target.index_dir.as_deref()).map_err(|e| {
        if e.starts_with(code_index::LIMIT_REFUSAL) {
            format!("{name}: {e}")
        } else {
            format!("{name}: the code index is unavailable: {e}")
        }
    })?;
    // A cursor binds to the index it paged: a re-index (an edited file)
    // changes the fingerprint, so an old cursor is refused, never re-sliced.
    let req = if enveloped {
        if let Some(map) = args.as_object_mut() {
            map.insert(
                ARG_INDEX.into(),
                json!(index.meta("fingerprint").unwrap_or_default()),
            );
        }
        Some(Request::take(name, &mut args).map_err(|e| {
            if e.contains("belongs to another query") {
                format!("{name}: this cursor belongs to another query, or the code index changed since its page (a file was edited and re-indexed); drop cursor to start over from the first page.")
            } else {
                e.replace("default 24576", "default 16384")
            }
        })?)
    } else {
        None
    };
    Ok((req, map, index))
}

/// Strict arguments: only the schema's names (a near miss gets a
/// did-you-mean), the required ones present, each of its type.
pub(crate) fn check_args(
    tool: &str,
    spec: &Value,
    args: &Value,
) -> Result<Map<String, Value>, String> {
    let Some(map) = args.as_object() else {
        return Err(format!("{tool}: arguments must be an object."));
    };
    let props = spec["inputSchema"]["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for key in map.keys() {
        if !props.contains_key(key) {
            let near = props
                .keys()
                .map(|k| (distance(&key.to_lowercase(), &k.to_lowercase()), k))
                .min()
                .filter(|(d, _)| *d <= 3);
            return Err(match near {
                Some((_, right)) => {
                    format!("{tool}: unknown argument \"{key}\"; did you mean \"{right}\"?")
                }
                None => format!(
                    "{tool}: unknown argument \"{key}\" (arguments: {}).",
                    props.keys().cloned().collect::<Vec<_>>().join(", ")
                ),
            });
        }
    }
    for req in spec["inputSchema"]["required"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let key = req.as_str().unwrap_or("");
        if map
            .get(key)
            .is_none_or(|v| v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty()))
        {
            return Err(format!("{tool}: `{key}` is required."));
        }
    }
    for (key, value) in map {
        let p = &props[key];
        let ok = match p["type"].as_str().unwrap_or("") {
            "string" => value.as_str().is_some_and(|s| enum_ok(p, s)),
            "boolean" => value.is_boolean(),
            "integer" => value.as_u64().is_some_and(|n| {
                p["minimum"].as_u64().is_none_or(|m| n >= m)
                    && p["maximum"].as_u64().is_none_or(|m| n <= m)
            }),
            "array" => value.as_array().is_some_and(|items| {
                items
                    .iter()
                    .all(|i| i.as_str().is_some_and(|s| enum_ok(&p["items"], s)))
            }),
            _ => true,
        };
        if !ok && !value.is_null() {
            return Err(format!("{tool}: `{key}` must be {}.", describe(p)));
        }
    }
    Ok(map.clone())
}

fn enum_ok(p: &Value, s: &str) -> bool {
    p["enum"]
        .as_array()
        .is_none_or(|e| e.iter().any(|v| v == s))
}

fn describe(p: &Value) -> String {
    match (
        p["type"].as_str().unwrap_or(""),
        p["enum"].as_array(),
        p["items"]["enum"].as_array(),
    ) {
        (_, Some(e), _) => format!("one of {}", join(e)),
        ("array", _, Some(e)) => format!("a list of {}", join(e)),
        ("integer", ..) => format!(
            "a whole number from {} to {}",
            p["minimum"].as_u64().unwrap_or(0),
            p["maximum"]
                .as_u64()
                .map_or("any".into(), |m| m.to_string())
        ),
        (t, ..) => format!("a {t}"),
    }
}

fn join(items: &[Value]) -> String {
    items
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut row = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            row[j + 1] = (prev[j] + usize::from(ca != *cb))
                .min(prev[j + 1] + 1)
                .min(row[j] + 1);
        }
        prev = row;
    }
    prev[b.len()]
}

// ── Shared pieces ───────────────────────────────────────────────────────

fn str_arg<'a>(args: &'a Map<String, Value>, key: &str) -> &'a str {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
}

/// Re-hashes the files a result returns, once each, through the confined
/// reader (CC3 review: a path an index names never steers a read).
struct Freshness {
    /// The canonical checkout root.
    root: PathBuf,
    seen: RefCell<HashMap<String, bool>>,
}

impl Freshness {
    fn new(root: &Path) -> Self {
        Freshness {
            root: std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()),
            seen: RefCell::new(HashMap::new()),
        }
    }

    fn stale(&self, path: &str, sha: &str) -> bool {
        if let Some(s) = self.seen.borrow().get(path) {
            return *s;
        }
        let now = code_index::read_index_path(&self.root, path).map(|b| code_index::short_sha(&b));
        let stale = now.is_none_or(|now| now != sha);
        self.seen.borrow_mut().insert(path.to_string(), stale);
        stale
    }

    fn any(&self) -> bool {
        self.seen.borrow().values().any(|s| *s)
    }
}

fn repo(index: &Index, root: &Path) -> Value {
    let list = |k: &str| -> Vec<String> {
        index
            .meta(k)
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default()
    };
    let mut out = json!({
        "head": index.meta("head").unwrap_or_default(),
        "fingerprint": index.meta("fingerprint").unwrap_or_default(),
        "indexedAt": index.meta("indexedAt").unwrap_or_default(),
        "dirtyFiles": code_index::dirty_files(root),
    });
    // CC3: the base commit read under this worktree's overlay, and how many
    // files the overlay replaces, adds or deletes.
    if index.overlaid() {
        let base = index.meta("base").unwrap_or_default();
        out["base"] = json!(base.get(..9).unwrap_or(&base));
        out["overlayFiles"] = json!(index
            .meta("overlayFiles")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0));
    }
    for (key, label) in [
        ("skipped", "skippedOverCap"),
        ("timedOut", "timedOut"),
        ("outsideRoot", "skippedOutsideRoot"),
    ] {
        let v = list(key);
        if !v.is_empty() {
            out[label] = json!(v);
        }
    }
    out
}

fn family(lang: &str) -> &'static str {
    code_index::parse::Lang::from_str(lang).map_or("", |l| l.family())
}

/// Definitions named `name`, narrowed by `qualifier` (container, a module
/// path part, or a path part) and `languages`.
fn targets(index: &Index, args: &Map<String, Value>) -> Result<Vec<NodeRow>, String> {
    let qualifier = str_arg(args, "qualifier");
    let langs = languages(args);
    let mut defs: Vec<NodeRow> = index
        .defs_named(str_arg(args, "name"))?
        .into_iter()
        .filter(|d| {
            qualifier.is_empty()
                || d.container == qualifier
                || d.modpath.split("::").any(|p| p == qualifier)
                || d.modpath == qualifier
                || d.path
                    .split('/')
                    .any(|p| p == qualifier || p.split('.').next() == Some(qualifier))
        })
        .filter(|d| langs.as_ref().is_none_or(|l| l.contains(family(&d.lang))))
        .collect();
    // Path and line order: `definitions` shows the first DEFINITIONS_SHOWN.
    defs.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.l0.cmp(&b.l0))
            .then(a.id.cmp(&b.id))
    });
    Ok(defs)
}

/// The first [`DEFINITIONS_SHOWN`] of `defs` (already sorted) for a result,
/// written into `out` with `definitionsTotal` and, past the cap, a hint.
fn put_definitions(out: &mut Value, defs: &[NodeRow]) {
    out["definitions"] = Value::Array(defs.iter().take(DEFINITIONS_SHOWN).map(def_json).collect());
    out["definitionsTotal"] = json!(defs.len());
    if defs.len() > DEFINITIONS_SHOWN {
        out["definitionsNote"] = json!(format!(
            "The first {DEFINITIONS_SHOWN} of {} definitions by path and line; add qualifier (an impl type, class or module) to narrow them.",
            defs.len()
        ));
    }
}

fn languages(args: &Map<String, Value>) -> Option<BTreeSet<String>> {
    args.get("languages").and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
}

fn def_json(d: &NodeRow) -> Value {
    let mut out = json!({ "path": d.path, "lines": [d.l0, d.l1], "kind": d.kind });
    if !d.container.is_empty() {
        out["container"] = json!(d.container);
    }
    if d.lang == "rust" && !d.modpath.is_empty() {
        out["module"] = json!(d.modpath);
    }
    if d.test {
        out["test"] = json!(true);
    }
    if d.command {
        out["tauriCommand"] = json!(true);
    }
    out
}

fn caller_json(src: &NodeRow) -> Value {
    let mut out = json!({ "name": src.name, "lines": [src.l0, src.l1] });
    if !src.container.is_empty() {
        out["container"] = json!(src.container);
    }
    if src.lang == "rust" && !src.modpath.is_empty() {
        out["module"] = json!(src.modpath);
    }
    out
}

/// Common-name advice (CC1 section 5, change 6).
fn advice(defs: &[NodeRow], args: &Map<String, Value>) -> Option<String> {
    (defs.len() > 2 && str_arg(args, "qualifier").is_empty()).then(|| {
        format!(
            "{} definitions are named {}: add qualifier (an impl type, class or module) so hits resolve to the one you mean.",
            defs.len(),
            str_arg(args, "name")
        )
    })
}

/// Nearest definition names by spelling, for an empty answer.
fn near_names(index: &Index, name: &str) -> Vec<String> {
    let lower = name.to_lowercase();
    let mut scored: Vec<(usize, String)> = index
        .def_names()
        .unwrap_or_default()
        .into_iter()
        .map(|n| (distance(&lower, &n.to_lowercase()), n))
        .filter(|(d, n)| *d <= (n.len() / 3).max(2))
        .collect();
    scored.sort();
    scored.into_iter().take(3).map(|(_, n)| n).collect()
}

/// Page `items` (already in order) with OB1's fit; `render` builds the
/// result from one page. Returns the content blocks.
fn paged(
    req: &Request,
    total: usize,
    fresh: &Freshness,
    render: &dyn Fn(std::ops::Range<usize>) -> Value,
) -> Result<Vec<Value>, String> {
    let listing = env::Listing {
        total,
        has_headers: false,
        row_limit: None,
        header_hint: "",
    };
    let offset = req.offset.min(total);
    let mut fitted = env::fit(
        req,
        &listing,
        || render(0..total).to_string(),
        |full, _| render(offset..offset + full).to_string(),
    )?;
    if fresh.any() {
        fitted.envelope.cap_reasons.push("stale_files");
        fitted.envelope.hint.push("A file changed since the index was built (stale:true); call again to re-index, or read it.".into());
    }
    Ok(vec![
        json!({ "type": "text", "text": fitted.text }),
        fitted.envelope.block(),
    ])
}

/// Group hit rows by file, keeping their order of first appearance.
fn by_file(rows: &[(String, String, bool, Value)], fresh: &Freshness) -> Value {
    let mut groups: Vec<Value> = Vec::new();
    let mut at: HashMap<&str, usize> = HashMap::new();
    for (path, sha, test, hit) in rows {
        let i = *at.entry(path.as_str()).or_insert_with(|| {
            let mut g = json!({ "path": path, "fileHash": sha, "hits": [] });
            if *test {
                g["test"] = json!(true);
            }
            if fresh.stale(path, sha) {
                g["stale"] = json!(true);
            }
            groups.push(g);
            groups.len() - 1
        });
        if let Some(hits) = groups[i]["hits"].as_array_mut() {
            hits.push(hit.clone());
        }
    }
    Value::Array(groups)
}

fn answer(
    name: &str,
    args: &Map<String, Value>,
    req: &Option<Request>,
    index: &Index,
    target: &Target,
) -> Result<Vec<Value>, String> {
    let root = target.root.as_path();
    let fresh = Freshness::new(root);
    let repo = repo(index, root);
    match (name, req) {
        ("read", _) => read(args, index, root, repo),
        (_, None) => Err("internal: no envelope".into()),
        ("map", Some(req)) => map(args, req, index, repo, &fresh),
        ("symbols", Some(req)) => symbols(args, req, index, repo, &fresh),
        ("callers", Some(req)) => callers(args, req, index, repo, &fresh),
        ("callees", Some(req)) => callees(args, req, index, repo, &fresh),
        ("impact", Some(req)) => impact(args, req, index, repo, &fresh),
        ("slice", Some(req)) => {
            slice::slice(args, req, index, repo, &fresh, target.index_dir.as_deref())
        }
        ("changes", Some(req)) => changes::changes(args, req, index, repo, &fresh, root),
        ("affected_tests", Some(req)) => {
            changes::affected_tests(args, req, index, repo, &fresh, root)
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

// ── The six tools ───────────────────────────────────────────────────────

fn map(
    args: &Map<String, Value>,
    req: &Request,
    index: &Index,
    repo: Value,
    fresh: &Freshness,
) -> Result<Vec<Value>, String> {
    let path = str_arg(args, "path").trim_matches('/').to_string();
    if let Some(file) = index.file(&path)? {
        let defs = index.defs_in(&path)?;
        let rows: Vec<Value> = defs
            .iter()
            .map(|d| {
                let mut v = json!({ "name": d.name, "kind": d.kind, "lines": [d.l0, d.l1] });
                if !d.container.is_empty() {
                    v["container"] = json!(d.container);
                }
                v
            })
            .collect();
        let render = |r: std::ops::Range<usize>| {
            let mut f = json!({ "lang": file.lang, "bytes": file.bytes, "fileHash": file.sha, "defs": defs.len() });
            if file.test {
                f["test"] = json!(true);
            }
            if fresh.stale(&file.path, &file.sha) {
                f["stale"] = json!(true);
            }
            json!({ "tool": "map", "path": path, "repo": repo, "file": f, "defs": rows[r].to_vec() })
        };
        return paged(req, rows.len(), fresh, &render);
    }
    let files = index.files_under(&path)?;
    if files.is_empty() {
        return Err(format!("no such indexed file or folder `{path}` (indexed: .rs .ts .tsx .py, tracked or untracked and not ignored; map with no path shows the root)."));
    }
    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{path}/")
    };
    let mut children: BTreeMap<String, (u64, i64, String)> = BTreeMap::new();
    for f in &files {
        let rest = &f.path[prefix.len()..];
        let (child, lang) = match rest.split_once('/') {
            Some((dir, _)) => (format!("{dir}/"), String::new()),
            None => (rest.to_string(), f.lang.clone()),
        };
        let e = children.entry(child).or_insert((0, 0, lang));
        e.0 += 1;
        e.1 += f.defs;
    }
    let rows: Vec<Value> = children
        .into_iter()
        .map(|(child, (n, defs, lang))| {
            if child.ends_with('/') {
                json!({ "name": child, "files": n, "defs": defs })
            } else {
                json!({ "name": child, "lang": lang, "defs": defs })
            }
        })
        .collect();
    let total_defs: i64 = files.iter().map(|f| f.defs).sum();
    let render = |r: std::ops::Range<usize>| json!({ "tool": "map", "path": path, "repo": repo, "totals": { "files": files.len(), "defs": total_defs }, "entries": rows[r].to_vec() });
    paged(req, rows.len(), fresh, &render)
}

fn symbols(
    args: &Map<String, Value>,
    req: &Request,
    index: &Index,
    repo: Value,
    fresh: &Freshness,
) -> Result<Vec<Value>, String> {
    let query = str_arg(args, "query");
    let kind = str_arg(args, "kind");
    let langs = languages(args);
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    let found: Vec<NodeRow> = index
        .search(query, 2_000)?
        .into_iter()
        .filter(|d| kind.is_empty() || d.kind == kind)
        .filter(|d| langs.as_ref().is_none_or(|l| l.contains(family(&d.lang))))
        .take(limit)
        .collect();
    let rows: Vec<Value> = found
        .iter()
        .map(|d| {
            let mut v = def_json(d);
            v["name"] = json!(d.name);
            v
        })
        .collect();
    let near = if rows.is_empty() {
        near_names(index, query)
    } else {
        Vec::new()
    };
    let render = |r: std::ops::Range<usize>| {
        let mut out =
            json!({ "tool": "symbols", "query": query, "repo": repo, "symbols": rows[r].to_vec() });
        if !near.is_empty() {
            out["didYouMean"] = json!(near);
        }
        out
    };
    paged(req, rows.len(), fresh, &render)
}

/// How one name-scan hit relates to the asked-about definitions.
fn status(e: &EdgeRow, ids: &[i64]) -> (bool, String) {
    match e.target {
        Some(t) if ids.contains(&t) => (true, e.how.clone()),
        Some(_) => (false, "resolves-elsewhere".into()),
        None => (false, e.how.clone()),
    }
}

fn callers(
    args: &Map<String, Value>,
    req: &Request,
    index: &Index,
    repo: Value,
    fresh: &Freshness,
) -> Result<Vec<Value>, String> {
    let name = str_arg(args, "name");
    let defs = targets(index, args)?;
    let ids: Vec<i64> = defs.iter().map(|d| d.id).collect();
    let fams: BTreeSet<String> = match languages(args) {
        Some(l) => l,
        None if defs.is_empty() => ["rust", "ts", "python"].map(String::from).into(),
        None => defs.iter().map(|d| family(&d.lang).to_string()).collect(),
    };
    let tests = args
        .get("includeTests")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let resolved_only = str_arg(args, "precision") == "resolved";
    let in_fams = |e: &EdgeRow| {
        fams.contains(family(&e.lang)) || (e.kind == "invoke" && fams.contains("rust"))
    };
    let named = index.edges_named(name)?;
    let other_languages = named
        .iter()
        .filter(|e| (tests || !e.test()) && !in_fams(e))
        .count();
    let mut hits: Vec<(bool, EdgeRow, String)> = named
        .into_iter()
        .filter(in_fams)
        .filter(|e| tests || !e.test())
        .map(|e| {
            let (ok, how) = status(&e, &ids);
            (ok, e, how)
        })
        .collect();
    let total = hits.len();
    let resolved = hits.iter().filter(|h| h.0).count();
    let elsewhere = hits.iter().filter(|h| h.2 == "resolves-elsewhere").count();
    if resolved_only {
        hits.retain(|h| h.0);
    }
    // Resolved first, then path and line: a cut loses the least certain.
    hits.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.path.cmp(&b.1.path))
            .then(a.1.line.cmp(&b.1.line))
    });
    let rows: Vec<(String, String, bool, Value)> = hits
        .iter()
        .map(|(ok, e, how)| {
            let mut h = json!({ "line": e.line, "kind": e.kind, "resolved": ok, "how": how });
            if !e.qual.is_empty() {
                h["qual"] = json!(e.qual);
            }
            if let (true, Some(t), true) = (*ok, e.target, defs.len() > 1) {
                match ids.iter().position(|i| *i == t) {
                    Some(at) if at < DEFINITIONS_SHOWN => h["def"] = json!(at),
                    Some(at) => h["defAt"] = json!(format!("{}:{}", defs[at].path, defs[at].l0)),
                    None => {}
                }
            }
            if let Some(src) = &e.src {
                h["caller"] = caller_json(src);
            }
            (e.path.clone(), e.sha.clone(), e.test(), h)
        })
        .collect();
    let target = json!({ "name": name, "qualifier": str_arg(args, "qualifier") });
    let advice = advice(&defs, args);
    let near = if defs.is_empty() && total == 0 {
        near_names(index, name)
    } else {
        Vec::new()
    };
    let render = |r: std::ops::Range<usize>| {
        let mut out = json!({ "tool": "callers", "target": target, "repo": repo });
        put_definitions(&mut out, &defs);
        out["languages"] = json!(fams);
        out["totals"] = json!({ "nameScanHits": total, "resolved": resolved, "unresolved": total - resolved, "resolvesElsewhere": elsewhere, "otherLanguages": other_languages });
        out["files"] = by_file(&rows[r], fresh);
        if let Some(a) = &advice {
            out["advice"] = json!(a);
        }
        if !near.is_empty() {
            out["didYouMean"] = json!(near);
        }
        out
    };
    paged(req, rows.len(), fresh, &render)
}

const CALL_KINDS: &[&str] = &["call", "method", "macro", "new", "jsx", "struct", "invoke"];

fn callees(
    args: &Map<String, Value>,
    req: &Request,
    index: &Index,
    repo: Value,
    fresh: &Freshness,
) -> Result<Vec<Value>, String> {
    let name = str_arg(args, "name");
    let defs = targets(index, args)?;
    if defs.is_empty() {
        return Err(not_found(index, name));
    }
    let mut rows: Vec<(String, String, bool, Value)> = Vec::new();
    let mut ends: HashMap<i64, Option<NodeRow>> = HashMap::new();
    for (di, d) in defs.iter().enumerate() {
        for e in index.edges_from(d.id)? {
            if !CALL_KINDS.contains(&e.kind.as_str()) {
                continue;
            }
            let mut h = json!({ "line": e.line, "kind": e.kind, "name": e.name, "resolved": e.target.is_some(), "how": e.how });
            if !e.qual.is_empty() {
                h["qual"] = json!(e.qual);
            }
            if defs.len() > 1 {
                h["from"] = json!(di);
            }
            if let Some(t) = e.target {
                let to = match ends.get(&t) {
                    Some(n) => n.clone(),
                    None => {
                        let n = index.node(t)?;
                        ends.insert(t, n.clone());
                        n
                    }
                };
                if let Some(to) = to {
                    h["to"] = def_json(&to);
                }
            }
            rows.push((e.path.clone(), e.sha.clone(), e.test(), h));
        }
    }
    let resolved = rows.iter().filter(|r| r.3["resolved"] == true).count();
    let total = rows.len();
    let advice = advice(&defs, args);
    let render = |r: std::ops::Range<usize>| {
        let mut out = json!({
            "tool": "callees", "target": { "name": name, "qualifier": str_arg(args, "qualifier") }, "repo": repo,
        });
        put_definitions(&mut out, &defs);
        out["totals"] = json!({ "calls": total, "resolved": resolved });
        out["files"] = by_file(&rows[r], fresh);
        if let Some(a) = &advice {
            out["advice"] = json!(a);
        }
        out
    };
    paged(req, rows.len(), fresh, &render)
}

fn not_found(index: &Index, name: &str) -> String {
    let near = near_names(index, name);
    if near.is_empty() {
        format!("no definition named `{name}` here (symbols finds names by their words).")
    } else {
        let quoted: Vec<String> = near.iter().map(|n| format!("`{n}`")).collect();
        format!(
            "no definition named `{name}` here; did you mean {}?",
            quoted.join(", ")
        )
    }
}

fn impact(
    args: &Map<String, Value>,
    req: &Request,
    index: &Index,
    repo: Value,
    fresh: &Freshness,
) -> Result<Vec<Value>, String> {
    let name = str_arg(args, "name");
    let defs = targets(index, args)?;
    if defs.is_empty() {
        return Err(not_found(index, name));
    }
    let depth = args.get("depth").and_then(Value::as_u64).unwrap_or(2);
    let tests = args
        .get("includeTests")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut seen: BTreeSet<i64> = defs.iter().map(|d| d.id).collect();
    let mut frontier: Vec<NodeRow> = defs.clone();
    let mut rows: Vec<(String, String, bool, Value)> = Vec::new();
    let mut fan_in: Vec<(usize, String, String)> = Vec::new();
    let mut unresolved = 0usize;
    for level in 1..=depth {
        let mut next: BTreeMap<i64, (NodeRow, usize)> = BTreeMap::new();
        for node in &frontier {
            let into = index.edges_to(node.id)?;
            fan_in.push((into.len(), node.name.clone(), node.container.clone()));
            unresolved += index
                .edges_named(&node.name)?
                .iter()
                .filter(|e| e.target.is_none() && family(&e.lang) == family(&node.lang))
                .count();
            for e in into {
                let test = e.test();
                let Some(src) = e.src else { continue };
                if (!tests && test) || seen.contains(&src.id) {
                    continue;
                }
                next.entry(src.id).or_insert((src, 0)).1 += 1;
            }
        }
        if seen.len() + next.len() > IMPACT_MAX_NODES + defs.len() {
            fan_in.sort_by_key(|f| std::cmp::Reverse(f.0));
            let busiest: Vec<String> = fan_in
                .iter()
                .take(3)
                .map(|(n, f, c)| {
                    if c.is_empty() {
                        format!("callers {{name:\"{f}\"}} ({n})")
                    } else {
                        format!("callers {{name:\"{f}\",qualifier:\"{c}\"}} ({n})")
                    }
                })
                .collect();
            return Err(format!(
                "more than {IMPACT_MAX_NODES} functions reach `{name}` by depth {level}, over the impact cap; ask a smaller depth, or callers on the busiest first: {}.",
                busiest.join(", ")
            ));
        }
        frontier = Vec::new();
        for (id, (src, via)) in next {
            seen.insert(id);
            let mut v =
                json!({ "depth": level, "name": src.name, "lines": [src.l0, src.l1], "via": via });
            if !src.container.is_empty() {
                v["container"] = json!(src.container);
            }
            rows.push((src.path.clone(), src.sha.clone(), src.test, v));
            frontier.push(src);
        }
        if frontier.is_empty() {
            break;
        }
    }
    let total = rows.len();
    let render = |r: std::ops::Range<usize>| {
        let mut out = json!({
            "tool": "impact", "target": { "name": name, "qualifier": str_arg(args, "qualifier") }, "repo": repo,
        });
        put_definitions(&mut out, &defs);
        out["depth"] = json!(depth);
        out["totals"] = json!({ "functions": total, "unresolvedHits": unresolved });
        out["note"] = json!("Only resolved calls are followed; unresolvedHits counts name-scan hits no rule tied to a definition (callers lists them).");
        out["files"] = by_file(&rows[r], fresh);
        out
    };
    paged(req, rows.len(), fresh, &render)
}

fn read(
    args: &Map<String, Value>,
    index: &Index,
    root: &Path,
    repo: Value,
) -> Result<Vec<Value>, String> {
    use crate::source_evidence::READ_MAX_LINES;
    let path = str_arg(args, "path").trim_start_matches("./").to_string();
    let Some(file) = index
        .file(&path)?
        .filter(|_| code_index::readable_path(&path))
    else {
        return Err(format!("no such indexed file `{path}` (map lists them; hidden, secret-looking, ignored and non-code files are never read)."));
    };
    let full = root.join(&path);
    let inside = std::fs::canonicalize(&full).is_ok_and(|c| c.starts_with(root));
    let link = std::fs::symlink_metadata(&full).map_or(true, |m| !m.file_type().is_file());
    if !inside || link {
        return Err(format!(
            "`{path}` is not a plain file inside the checkout; nothing was read."
        ));
    }
    let canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let bytes = code_index::read_index_path(&canon, &path)
        .ok_or_else(|| format!("could not read `{path}`; nothing was read."))?;
    let stale = code_index::short_sha(&bytes) != file.sha;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let (mut from, mut to) = (
        args.get("fromLine").and_then(Value::as_u64).unwrap_or(1) as usize,
        args.get("toLine")
            .and_then(Value::as_u64)
            .map(|t| t as usize),
    );
    let symbol = str_arg(args, "symbol");
    if !symbol.is_empty() {
        let def = index
            .defs_in(&path)?
            .into_iter()
            .find(|d| d.name == symbol)
            .ok_or_else(|| {
                format!("no definition named `{symbol}` in `{path}` (map {{path}} lists them).")
            })?;
        (from, to) = (def.l0 as usize, Some(def.l1 as usize));
    }
    let n = lines.len();
    if from > n.max(1) {
        return Err(format!(
            "`fromLine` {from} is past the end: `{path}` has {n} lines."
        ));
    }
    let to = to.unwrap_or(n).min(n);
    // The budget counts the text as JSON escapes it, so the whole result
    // stays near DEFAULT_MAX_BYTES and far under the envelope's hard max.
    let mut body = String::new();
    let mut spent = 0usize;
    let mut last = from.saturating_sub(1);
    let mut cut_line = None;
    for (i, line) in lines
        .iter()
        .enumerate()
        .take(to)
        .skip(from.saturating_sub(1))
    {
        let numbered = format!("{}\t{line}\n", i + 1);
        let cost = escaped_len(&numbered);
        if last + 1 - from >= READ_MAX_LINES || spent + cost > DEFAULT_MAX_BYTES {
            if body.is_empty() {
                // One line over the whole budget: its start, flagged, so
                // `more` still moves on.
                let head = cut_to(&numbered, DEFAULT_MAX_BYTES.saturating_sub(2));
                body.push_str(head);
                body.push('\n');
                cut_line = Some(i + 1);
                last = i + 1;
            }
            break;
        }
        body.push_str(&numbered);
        spent += cost;
        last = i + 1;
    }
    let mut out = json!({
        "tool": "read", "path": path, "repo": repo, "lines": [from, last], "totalLines": n,
        "fileHash": file.sha, "text": body,
        "note": crate::untrusted::json_note(crate::untrusted::Origin::File),
    });
    if stale {
        out["stale"] = json!(true);
    }
    if let Some(line) = cut_line {
        out["truncatedLine"] = json!(line);
        out["truncatedNote"] = json!(format!(
            "Line {line} is longer than the {DEFAULT_MAX_BYTES}-byte budget; only its start is shown."
        ));
    }
    if last < to {
        out["more"] = json!(format!(
            "lines {}-{to}: call read with fromLine {}",
            last + 1,
            last + 1
        ));
    }
    Ok(vec![json!({ "type": "text", "text": out.to_string() })])
}

/// Bytes `text` takes inside a JSON string.
fn escaped_len(text: &str) -> usize {
    text.chars().map(escaped_char_len).sum()
}

fn escaped_char_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        c if (c as u32) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// The longest start of `text` whose escaped length is at most `budget`,
/// cut on a character boundary.
fn cut_to(text: &str, budget: usize) -> &str {
    let mut spent = 0;
    for (at, c) in text.char_indices() {
        spent += escaped_char_len(c);
        if spent > budget {
            return &text[..at];
        }
    }
    text
}

#[path = "code_context_slice.rs"]
mod slice;

#[path = "code_context_changes.rs"]
pub(crate) mod changes;

#[cfg(test)]
#[path = "code_context_tests.rs"]
pub(crate) mod tests;
