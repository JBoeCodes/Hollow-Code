//! `slice` (CC4): a deterministic list of cards for a named symbol, or for
//! the definitions a task's words match best, plus its callers and callees
//! to `depth`. One card per definition: path, kind, container, line range,
//! signature, the file's hash, a hash of the definition's own lines
//! (`bodyHash`) and a hash of the card's content (`cardHash`, which leaves
//! out the line range and the file hash, so an edit elsewhere in a file
//! never changes it).
//!
//! Each answer carries a `handle` (`h1.<16 hex>`): a hash of the checkout,
//! the arguments and the card list, whose card hashes and line ranges are
//! kept in the index folder's `slices/` (at most [`HANDLES_KEPT`], each
//! leased [`HANDLE_LEASE`] from its last use). Called again with it, slice
//! returns only added and changed cards, `moved` (same content, new lines)
//! and `removed` ids, or `{"notModified":true,"handle":...}` when nothing
//! changed. Cards page with the envelope cursor like every other list here,
//! never cut mid-card. A child of `code_context`, so it shares its helpers.

use super::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Hops of callers and callees at most.
pub(super) const SLICE_MAX_DEPTH: u64 = 2;
/// A slice of more definitions than this is refused.
pub(crate) const SLICE_MAX_CARDS: usize = 300;
/// A symbol's definitions taken as targets (by path and line).
const SYMBOL_TARGETS: usize = 5;
/// A task's best-matching definitions taken as targets.
const TASK_TARGETS: usize = 3;
/// A handle's state is kept this long after its last use.
pub(crate) const HANDLE_LEASE: Duration = Duration::from_secs(30 * 60);
/// At most this many handles' states are kept (the least recently used go).
pub(crate) const HANDLES_KEPT: usize = 64;
const HANDLE_VERSION: &str = "h1";
const SIGNATURE_MAX_LINES: usize = 4;
const SIGNATURE_MAX_CHARS: usize = 300;
/// A delta's `moved` and `removed` each list at most this many entries,
/// and together at most half the page's `maxBytes`.
pub(crate) const DELTA_LIST_MAX: usize = 100;
/// Makes each save's temp file name unique within this process, so two
/// calls saving the same handle never write one temp file.
static SAVE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Words a task's text uses that name no code.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "do", "fix", "for", "from", "how", "in",
    "into", "is", "it", "its", "make", "of", "on", "or", "so", "that", "the", "this", "to", "when",
    "where", "which", "why", "with",
];

const HANDLE_REFUSAL: &str = "this handle is not one a slice gave here, or its 30-minute lease ran out; drop handle to get the whole slice again.";

/// One card before it is written out.
struct Card {
    id: String,
    hash: String,
    l0: i64,
    l1: i64,
    json: Value,
}

/// What a handle keeps: the arguments' hash and each card's id, hash and
/// lines, in card order.
#[derive(serde::Serialize, serde::Deserialize)]
struct State {
    v: u32,
    args: String,
    cards: Vec<(String, String, i64, i64)>,
}

pub(super) fn slice(
    args: &Map<String, Value>,
    req: &Request,
    index: &Index,
    repo: Value,
    fresh: &Freshness,
    index_dir: Option<&Path>,
) -> Result<Vec<Value>, String> {
    let symbol = str_arg(args, "symbol");
    let task = str_arg(args, "task");
    match (symbol.is_empty(), task.is_empty()) {
        (true, true) => return Err("`symbol` or `task` is required.".into()),
        (false, false) => {
            return Err("`task` must be left out when `symbol` is given (pass one).".into())
        }
        (true, false) if !str_arg(args, "qualifier").is_empty() => {
            return Err("`qualifier` must be left out with `task` (it narrows a symbol).".into())
        }
        _ => {}
    }
    let depth = args.get("depth").and_then(Value::as_u64).unwrap_or(1);
    let store = cache_dir(index_dir);
    let args_key = args_key(&fresh.root, args, depth);
    // A handle is checked before any work: a bad one is refused whatever
    // the slice would be.
    let since = match str_arg(args, "handle") {
        "" => None,
        h => Some((h.to_string(), load(store.as_deref(), h, &args_key)?)),
    };
    let (targets, target_json, note) = if symbol.is_empty() {
        task_targets(index, args, task)?
    } else {
        symbol_targets(index, args, symbol)?
    };
    let cards = cards(index, &targets, depth, fresh)?;
    let handle = handle_of(&args_key, &cards);
    save(store.as_deref(), &handle, &args_key, &cards);
    let head = |out: &mut Value| {
        out["tool"] = json!("slice");
        out["target"] = target_json.clone();
        out["repo"] = repo.clone();
        out["depth"] = json!(depth);
        out["handle"] = json!(handle);
        if let Some(n) = &note {
            out["note"] = json!(n);
        }
    };
    let Some((old_handle, old)) = since else {
        let total = cards.len();
        let render = |r: std::ops::Range<usize>| {
            let mut out = json!({});
            head(&mut out);
            out["totals"] = json!({ "cards": total, "targets": targets.len() });
            out["cards"] = Value::Array(cards[r].iter().map(|c| c.json.clone()).collect());
            out
        };
        return paged(req, total, fresh, &render);
    };
    // The delta against the handle's cards.
    let before: HashMap<&str, (&str, i64, i64)> = old
        .cards
        .iter()
        .map(|(id, h, l0, l1)| (id.as_str(), (h.as_str(), *l0, *l1)))
        .collect();
    let now: BTreeSet<&str> = cards.iter().map(|c| c.id.as_str()).collect();
    let mut sent: Vec<Value> = Vec::new();
    let mut moved: Vec<Value> = Vec::new();
    let (mut added, mut changed) = (0usize, 0usize);
    for c in &cards {
        match before.get(c.id.as_str()) {
            None => {
                added += 1;
                let mut v = c.json.clone();
                v["change"] = json!("added");
                sent.push(v);
            }
            Some((h, _, _)) if *h != c.hash => {
                changed += 1;
                let mut v = c.json.clone();
                v["change"] = json!("changed");
                sent.push(v);
            }
            Some((_, l0, l1)) if (*l0, *l1) != (c.l0, c.l1) => {
                moved.push(json!({ "id": c.id, "lines": [c.l0, c.l1] }));
            }
            Some(_) => {}
        }
    }
    let removed: Vec<&str> = old
        .cards
        .iter()
        .map(|(id, ..)| id.as_str())
        .filter(|id| !now.contains(id))
        .collect();
    if sent.is_empty() && moved.is_empty() && removed.is_empty() {
        let text = json!({ "notModified": true, "handle": handle });
        return Ok(vec![json!({ "type": "text", "text": text.to_string() })]);
    }
    let total = sent.len();
    // Moves and removals ride on the first page, cut to a share of the
    // budget so that page stays within maxBytes.
    let share = req.max_bytes / 4;
    let moved_listed = listed(&moved, share);
    let removed_listed = listed(&removed, share);
    let cut = (moved_listed < moved.len() || removed_listed < removed.len()).then(|| {
        format!(
            "moved lists {moved_listed} of {} and removed {removed_listed} of {}; drop handle for the whole slice.",
            moved.len(),
            removed.len()
        )
    });
    let render = |r: std::ops::Range<usize>| {
        let first = r.start == 0;
        let mut out = json!({});
        head(&mut out);
        out["since"] = json!(old_handle);
        out["totals"] = json!({
            "cards": cards.len(), "added": added, "changed": changed,
            "moved": moved.len(), "removed": removed.len(),
        });
        out["cards"] = Value::Array(sent[r].to_vec());
        if first {
            out["moved"] = json!(&moved[..moved_listed]);
            out["removed"] = json!(&removed[..removed_listed]);
            if let Some(c) = &cut {
                out["listsCut"] = json!(c);
            }
        }
        out
    };
    paged(req, total, fresh, &render)
}

/// How many of `items`, at most [`DELTA_LIST_MAX`], fit in `bytes` as
/// JSON.
fn listed<T: serde::Serialize>(items: &[T], bytes: usize) -> usize {
    let mut used = 2;
    for (i, item) in items.iter().take(DELTA_LIST_MAX).enumerate() {
        used += serde_json::to_string(item).map_or(0, |t| t.len()) + 1;
        if used > bytes {
            return i;
        }
    }
    items.len().min(DELTA_LIST_MAX)
}

/// The checkout and the slice's arguments as they are used (not `handle`;
/// `maxBytes` and `cursor` are already out): trimmed text, the resolved
/// depth and the sorted languages, so `{"symbol":"a"}` and
/// `{"symbol":" a ","depth":1}` are one slice with one handle.
fn args_key(root: &Path, args: &Map<String, Value>, depth: u64) -> String {
    let own = json!({
        "symbol": str_arg(args, "symbol"),
        "qualifier": str_arg(args, "qualifier"),
        "task": str_arg(args, "task"),
        "depth": depth,
        "languages": languages(args),
    });
    format!(
        "slice\n{}\n{}",
        root.to_string_lossy(),
        env::canonical(&own)
    )
}

fn handle_of(args_key: &str, cards: &[Card]) -> String {
    let mut text = format!("{args_key}\n");
    for c in cards {
        text.push_str(&format!("{}\t{}\t{}\t{}\n", c.id, c.hash, c.l0, c.l1));
    }
    let full = code_index::hex(&Sha256::digest(text.as_bytes()));
    format!("{HANDLE_VERSION}.{}", &full[..16])
}

/// The folder handles' states live in.
fn cache_dir(index_dir: Option<&Path>) -> Option<PathBuf> {
    code_index::cache_dir(index_dir).map(|d| d.join("slices"))
}

/// `h1.<16 hex>` and nothing else, so a handle never names a path.
fn handle_file(dir: &Path, handle: &str) -> Option<PathBuf> {
    let id = handle.strip_prefix(HANDLE_VERSION)?.strip_prefix('.')?;
    let ok = id.len() == 16
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    ok.then(|| dir.join(format!("{id}.json")))
}

/// The handle's state, its lease refreshed; refused when it is malformed,
/// unknown, expired or from other arguments.
fn load(dir: Option<&Path>, handle: &str, args_key: &str) -> Result<State, String> {
    let file = dir
        .and_then(|d| handle_file(d, handle))
        .ok_or_else(|| HANDLE_REFUSAL.to_string())?;
    let meta = std::fs::symlink_metadata(&file).map_err(|_| HANDLE_REFUSAL.to_string())?;
    let expired = meta
        .modified()
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .is_some_and(|age| age > HANDLE_LEASE);
    if !meta.file_type().is_file() || expired {
        let _ = std::fs::remove_file(&file);
        return Err(HANDLE_REFUSAL.into());
    }
    let state: State = std::fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|s: &State| s.v == 1)
        .ok_or_else(|| HANDLE_REFUSAL.to_string())?;
    if state.args != code_index::short_sha(args_key.as_bytes()) {
        return Err("this handle belongs to another slice (another symbol, task, depth or checkout); drop handle, or repeat the arguments it was given with.".into());
    }
    touch(&file);
    Ok(state)
}

fn touch(file: &Path) {
    if let Ok(f) = std::fs::OpenOptions::new().append(true).open(file) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Keep this answer's state under its handle (a temp file, then a rename),
/// and prune: expired states, then the least recently used past the cap.
/// Best effort: a slice still answers when the folder can't be written,
/// and the next call with its handle is refused and starts over.
fn save(dir: Option<&Path>, handle: &str, args_key: &str, cards: &[Card]) {
    let Some(dir) = dir else { return };
    let Some(file) = handle_file(dir, handle) else {
        return;
    };
    if file.is_file() {
        touch(&file);
    } else {
        let state = State {
            v: 1,
            args: code_index::short_sha(args_key.as_bytes()),
            cards: cards
                .iter()
                .map(|c| (c.id.clone(), c.hash.clone(), c.l0, c.l1))
                .collect(),
        };
        let Ok(bytes) = serde_json::to_vec(&state) else {
            return;
        };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let seq = SAVE_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".{handle}.{}.{seq}.tmp", std::process::id()));
        if std::fs::write(&tmp, bytes).is_err() || std::fs::rename(&tmp, &file).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return;
        }
    }
    prune(dir);
}

fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    let mut kept: Vec<(SystemTime, PathBuf)> = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Some(m) = e.metadata().ok().and_then(|m| m.modified().ok()) else {
            continue;
        };
        if now.duration_since(m).is_ok_and(|age| age > HANDLE_LEASE) {
            let _ = std::fs::remove_file(&path);
        } else {
            kept.push((m, path));
        }
    }
    if kept.len() > HANDLES_KEPT {
        kept.sort();
        for (_, path) in &kept[..kept.len() - HANDLES_KEPT] {
            let _ = std::fs::remove_file(path);
        }
    }
}

type Targets = (Vec<NodeRow>, Value, Option<String>);

fn symbol_targets(
    index: &Index,
    args: &Map<String, Value>,
    symbol: &str,
) -> Result<Targets, String> {
    let mut named = args.clone();
    named.insert("name".into(), json!(symbol));
    let mut defs = targets(index, &named)?;
    if defs.is_empty() {
        return Err(not_found(index, symbol));
    }
    let note = (defs.len() > SYMBOL_TARGETS).then(|| {
        format!(
            "{} definitions are named {symbol}; the first {SYMBOL_TARGETS} by path and line are the targets: add qualifier (an impl type, class or module) to pick one.",
            defs.len()
        )
    });
    defs.truncate(SYMBOL_TARGETS);
    let target = json!({ "symbol": symbol, "qualifier": str_arg(args, "qualifier") });
    Ok((defs, target, note))
}

/// The definitions a task's words match best: each word through the name
/// search (split identifiers, a prefix each), scored by how many words a
/// definition matches, then non-test first, shorter names, path and line.
fn task_targets(index: &Index, args: &Map<String, Value>, task: &str) -> Result<Targets, String> {
    let lower = task.to_lowercase();
    // Each word once, in first-seen order: a repeat must not weigh more.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty() && !STOPWORDS.contains(w) && seen.insert(w))
        .collect();
    words.truncate(12);
    if words.is_empty() {
        return Err("`task` needs a word that names code (only common words were given).".into());
    }
    let langs = languages(args);
    let mut score: BTreeMap<i64, (usize, NodeRow)> = BTreeMap::new();
    for w in &words {
        for d in index.search(w, 500)? {
            if langs.as_ref().is_none_or(|l| l.contains(family(&d.lang))) {
                score.entry(d.id).or_insert((0, d)).0 += 1;
            }
        }
    }
    let mut ranked: Vec<(usize, NodeRow)> = score.into_values().collect();
    ranked.sort_by(|(sa, a), (sb, b)| {
        sb.cmp(sa)
            .then(a.test.cmp(&b.test))
            .then(a.name.len().cmp(&b.name.len()))
            .then(a.path.cmp(&b.path))
            .then(a.l0.cmp(&b.l0))
            .then(a.id.cmp(&b.id))
    });
    if ranked.is_empty() {
        return Err(format!(
            "no such definition matches the task's words ({}); symbols finds names by their words.",
            words.join(", ")
        ));
    }
    let defs: Vec<NodeRow> = ranked
        .into_iter()
        .take(TASK_TARGETS)
        .map(|(_, d)| d)
        .collect();
    let matched: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    let target = json!({ "task": task, "matched": matched });
    Ok((defs, target, None))
}

/// The targets, their callers and their callees to `depth` hops (resolved
/// calls only), as cards in order: targets, then by distance, path, line.
fn cards(
    index: &Index,
    targets: &[NodeRow],
    depth: u64,
    fresh: &Freshness,
) -> Result<Vec<Card>, String> {
    // id -> (node, distance, caller, callee)
    let mut found: BTreeMap<i64, (NodeRow, u64, bool, bool)> = BTreeMap::new();
    for t in targets {
        found.insert(t.id, (t.clone(), 0, false, false));
    }
    for callers in [true, false] {
        let mut seen: BTreeSet<i64> = targets.iter().map(|t| t.id).collect();
        let mut frontier: Vec<NodeRow> = targets.to_vec();
        for level in 1..=depth {
            let mut next: BTreeMap<i64, NodeRow> = BTreeMap::new();
            for node in &frontier {
                let reached: Vec<NodeRow> = if callers {
                    index
                        .edges_to(node.id)?
                        .into_iter()
                        .filter_map(|e| e.src)
                        .collect()
                } else {
                    let mut out = Vec::new();
                    for e in index.edges_from(node.id)? {
                        if !CALL_KINDS.contains(&e.kind.as_str()) {
                            continue;
                        }
                        if let Some(n) = e.target.map(|t| index.node(t)).transpose()?.flatten() {
                            out.push(n);
                        }
                    }
                    out
                };
                for n in reached {
                    if seen.insert(n.id) {
                        next.insert(n.id, n);
                    }
                }
            }
            for (id, n) in &next {
                let e = found.entry(*id).or_insert((n.clone(), level, false, false));
                if e.1 != 0 {
                    e.1 = e.1.min(level);
                    if callers {
                        e.2 = true;
                    } else {
                        e.3 = true;
                    }
                }
            }
            if found.len() > SLICE_MAX_CARDS {
                return Err(format!(
                    "more than {SLICE_MAX_CARDS} definitions are in this slice by depth {level}, over the slice cap; ask depth 1, or add qualifier (or use callers and impact on the busiest)."
                ));
            }
            frontier = next.into_values().collect();
            if frontier.is_empty() {
                break;
            }
        }
    }
    let mut nodes: Vec<(NodeRow, u64, bool, bool)> = found.into_values().collect();
    nodes.sort_by(|a, b| (a.1, &a.0.path, a.0.l0, a.0.id).cmp(&(b.1, &b.0.path, b.0.l0, b.0.id)));
    let mut texts: HashMap<String, Option<String>> = HashMap::new();
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();
    for (n, distance, caller, callee) in nodes {
        let text = texts
            .entry(n.path.clone())
            .or_insert_with(|| {
                code_index::read_index_path(&fresh.root, &n.path)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
            })
            .as_deref();
        let lines: Vec<&str> = text
            .map(|t| {
                t.lines()
                    .skip((n.l0 - 1).max(0) as usize)
                    .take((n.l1 - n.l0 + 1).max(0) as usize)
                    .collect()
            })
            .unwrap_or_default();
        let base = if n.container.is_empty() {
            format!("{}:{}", n.path, n.name)
        } else {
            format!("{}:{}::{}", n.path, n.container, n.name)
        };
        let seen = ids.entry(base.clone()).or_insert(0);
        *seen += 1;
        let id = if *seen == 1 {
            base
        } else {
            format!("{base}#{seen}")
        };
        let rel = match (distance, caller, callee) {
            (0, ..) => "target",
            (_, true, true) => "both",
            (_, true, false) => "caller",
            _ => "callee",
        };
        let mut card = def_json(&n);
        card["id"] = json!(id);
        card["name"] = json!(n.name);
        card["rel"] = json!(rel);
        card["distance"] = json!(distance);
        card["signature"] = json!(signature(&lines));
        card["bodyHash"] = json!(if text.is_some() {
            code_index::short_sha(lines.join("\n").as_bytes())
        } else {
            String::new()
        });
        // The card's own content: not its lines (a move is reported apart)
        // nor its file's hash (an edit elsewhere in the file).
        let mut own = card.clone();
        if let Some(m) = own.as_object_mut() {
            m.remove("lines");
        }
        let hash = code_index::short_sha(env::canonical(&own).as_bytes());
        card["cardHash"] = json!(hash);
        card["fileHash"] = json!(n.sha);
        if fresh.stale(&n.path, &n.sha) {
            card["stale"] = json!(true);
        }
        out.push(Card {
            id,
            hash,
            l0: n.l0,
            l1: n.l1,
            json: card,
        });
    }
    Ok(out)
}

/// The definition's first lines up to its body (`{`) or the end of a
/// Python header (`:`), at most [`SIGNATURE_MAX_LINES`] lines and
/// [`SIGNATURE_MAX_CHARS`] characters.
fn signature(lines: &[&str]) -> String {
    let mut sig: Vec<&str> = Vec::new();
    for line in lines.iter().take(SIGNATURE_MAX_LINES) {
        if let Some((head, _)) = line.split_once('{') {
            sig.push(head);
            break;
        }
        sig.push(line);
        let t = line.trim_end();
        if t.ends_with(':') || t.ends_with(';') || t.ends_with("=>") {
            break;
        }
    }
    let joined = sig
        .iter()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    let joined = joined.trim_end();
    match joined.char_indices().nth(SIGNATURE_MAX_CHARS) {
        Some((at, _)) => format!("{}...", &joined[..at]),
        None => joined.to_string(),
    }
}

#[cfg(test)]
#[path = "code_context_slice_tests.rs"]
mod tests;
