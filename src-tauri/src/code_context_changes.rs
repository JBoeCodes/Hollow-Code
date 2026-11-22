//! `changes` and `affected_tests` (CC5): from a diff to the definitions it
//! touches, and from those to the tests that cover them, so a lane can run
//! a lean local check before the full gate.
//!
//! `changes` diffs the work tree (and its untracked, unignored files)
//! against `base` through the hardened git (`code_index::Git`: no external
//! diff, no textconv, stdin closed) and maps each hunk to the definitions
//! whose lines it touches: on the new side from the index, on the old side
//! by parsing the base's blob in process (nothing runs). A definition only
//! the old side has is `deleted`, one only the new side has is `added`;
//! a rename is a delete plus an add. Files the index can't map (other
//! languages, config, fixtures, generated or binary files) are listed under
//! `unmapped` with a reason.
//!
//! `affected_tests` walks callers from those definitions (resolved calls,
//! test helpers for free, `depth` levels of non-test callers), plus rare
//! names' unresolved calls (`name-match`), each Rust test module in or
//! attached to a changed file (`same-file`, `same-module`) and TypeScript
//! test files importing a changed file (`import`). Rust tests come back as
//! `cargo test --lib` filters (the module path follows `#[path]` attributes),
//! grouped to the narrowest module; TypeScript (vitest, or Playwright for
//! `.spec.` files) and Python tests as files.
//! What it can't trace is listed in `cannotMap`. It never claims to be
//! complete: `missRate` is the measured rate and the full gate stays whole.
//! A child of `code_context`, so it shares its helpers.

use super::*;
use crate::code_index::parse::{self, Lang};
use crate::code_index::Git;
use std::collections::{HashSet, VecDeque};

/// Levels of non-test callers `affected_tests` follows at most.
pub(super) const AFFECTED_MAX_DEPTH: u64 = 4;
/// A diff of more changed files than this is refused.
pub(crate) const MAX_CHANGED_FILES: usize = 2_000;
/// `paths` and `symbols` hold at most this many.
const MAX_LIST_ARG: usize = 100;
/// A file row of `changes` lists at most this many symbols.
const SYMBOLS_PER_FILE: usize = 100;
/// A test group lists at most this many tests by name.
const TESTS_PER_GROUP: usize = 20;
/// `cannotMap` lists at most this many entries (all are counted).
const CANNOT_MAP_SHOWN: usize = 15;
/// Functions one `affected_tests` walk reaches at most.
const REACH_CAP: usize = 2_000;
/// Unresolved calls to a name are followed only when at most this many
/// definitions share it.
const NAME_MATCH_MAX_DEFS: usize = 3;
/// Files the TypeScript import walk visits at most.
const IMPORT_FILES_CAP: usize = 200;
/// Patch paths per `git diff` call.
const PATCH_CHUNK: usize = 200;

/// The measured miss rate (docs/CODE_CONTEXT.md, "affected_tests evaluation").
pub(crate) const MISS_RATE: &str = "Measured 2026-11-18 on 10 merged Hollow PRs: 0 of the 63 tests they added or changed went unlisted (2 only through same-file or same-module filters), and 3 breaks on purpose had all 16 failing tests listed. A small sample of tests near each change: expect misses elsewhere.";

/// Definition kinds that hold other definitions: dropped from a hunk's
/// symbols when a definition inside them is touched too.
const CONTAINER_KINDS: &[&str] = &["mod", "trait", "class", "iface"];

// ── The diff ────────────────────────────────────────────────────────────

/// One changed definition.
#[derive(Debug, Clone)]
pub(crate) struct ChangedSymbol {
    /// `edited`, `added`, `deleted`, or `named` (an `affected_tests`
    /// `symbols` argument).
    pub change: &'static str,
    /// The definition now (not for `deleted`).
    pub node: Option<NodeRow>,
    /// The base's definition (`deleted`).
    pub old: Option<parse::Def>,
    pub path: String,
}

impl ChangedSymbol {
    fn name(&self) -> &str {
        match (&self.node, &self.old) {
            (Some(n), _) => &n.name,
            (None, Some(o)) => &o.name,
            _ => "",
        }
    }

    #[cfg(test)]
    fn test(&self) -> bool {
        match (&self.node, &self.old) {
            (Some(n), _) => n.test,
            (None, Some(o)) => o.test,
            _ => false,
        }
    }

    fn json(&self) -> Value {
        let mut v = match (&self.node, &self.old) {
            (Some(n), _) => {
                let mut v = def_json(n);
                v["name"] = json!(n.name);
                v
            }
            (None, Some(o)) => {
                let mut v = json!({ "name": o.name, "kind": o.kind, "oldLines": [o.l0, o.l1] });
                if !o.container.is_empty() {
                    v["container"] = json!(o.container);
                }
                if o.test {
                    v["test"] = json!(true);
                }
                v
            }
            _ => json!({}),
        };
        if let Some(m) = v.as_object_mut() {
            m.remove("path");
        }
        v["change"] = json!(self.change);
        v
    }
}

/// One changed file.
#[derive(Debug, Clone, Default)]
pub(crate) struct ChangedFile {
    pub path: String,
    /// `modified`, `added` or `deleted`.
    pub status: &'static str,
    /// The index's hash of the file now (empty when it has none).
    pub sha: String,
    pub symbols: Vec<ChangedSymbol>,
    /// Changed lines (new side, or old side for a pure deletion) outside
    /// every definition.
    pub outside: u64,
    /// Why the file is not mapped to definitions.
    pub unmapped: Option<String>,
    pub test: bool,
}

/// A hunk's old and new line ranges (`@@ -a,b +c,d @@`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hunk {
    pub old: (i64, i64),
    pub new: (i64, i64),
}

/// Why a file the index can't map is left out, by its path.
pub(crate) fn unmapped_reason(path: &str) -> String {
    let lower = path.to_lowercase();
    let file = lower.rsplit('/').next().unwrap_or(&lower);
    let ext = file.rsplit_once('.').map_or("", |(_, e)| e);
    let in_dir = |d: &str| lower.split('/').any(|p| p == d);
    if in_dir("fixtures") || in_dir("__fixtures__") || in_dir("testdata") || in_dir("snapshots") {
        return "fixture: tests that read it are not traced".into();
    }
    if in_dir("generated")
        || in_dir("dist")
        || file.contains(".generated.")
        || file.ends_with(".d.ts")
        || file.ends_with(".lock")
        || file == "package-lock.json"
    {
        return "generated: not traced".into();
    }
    match ext {
        "mjs" | "js" | "cjs" | "jsx" => {
            "not indexed: .mjs and .js files are left out of the index".into()
        }
        "toml" | "json" | "jsonc" | "yaml" | "yml" | "plist" | "ini" | "conf" | "cfg" | "env"
        | "entitlements" | "tsv" | "csv" => {
            "config or data: tests that read it are not traced".into()
        }
        "png" | "jpg" | "jpeg" | "gif" | "ico" | "icns" | "pdf" | "woff" | "woff2" | "ttf"
        | "otf" | "wasm" | "gz" | "zip" | "mp3" | "mp4" | "wav" | "svg" => {
            "binary or asset: not traced".into()
        }
        "md" | "txt" => "docs: not traced".into(),
        "css" | "html" => "styles or markup: not indexed".into(),
        "sh" | "bash" | "zsh" => "script: not indexed".into(),
        _ => "not an indexed language (.rs .ts .tsx .py are)".into(),
    }
}

/// A git-quoted path (`"a\tb"`, octal escapes) as written, else as is.
fn unquote(s: &str) -> String {
    let Some(inner) = s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) else {
        return s.to_string();
    };
    let bytes = inner.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let c = bytes[i + 1];
        let simple = match c {
            b'n' => Some(b'\n'),
            b't' => Some(b'\t'),
            b'r' => Some(b'\r'),
            b'a' => Some(7),
            b'b' => Some(8),
            b'f' => Some(12),
            b'v' => Some(11),
            b'\\' => Some(b'\\'),
            b'"' => Some(b'"'),
            _ => None,
        };
        if let Some(b) = simple {
            out.push(b);
            i += 2;
        } else if i + 3 < bytes.len()
            && bytes[i + 1..i + 4]
                .iter()
                .all(|d| (b'0'..=b'7').contains(d))
        {
            let v = (bytes[i + 1] - b'0') as u32 * 64
                + (bytes[i + 2] - b'0') as u32 * 8
                + (bytes[i + 3] - b'0') as u32;
            out.push(v as u8);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse `git diff -U0` output into each file's hunks and whether git
/// called it binary, by path (the new side's, else the old side's).
pub(crate) fn parse_patch(text: &str) -> HashMap<String, (Vec<Hunk>, bool)> {
    let mut out: HashMap<String, (Vec<Hunk>, bool)> = HashMap::new();
    let mut current: Option<String> = None;
    let mut old_path: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // `a/X b/X` (no renames, so both sides name X).
            let path = if rest.starts_with('"') {
                rest.split("\" \"")
                    .nth(1)
                    .map(|b| unquote(&format!("\"{b}")))
            } else {
                let n = rest.len().saturating_sub(5) / 2;
                rest.get(2..2 + n).map(str::to_string)
            };
            current = path.map(|p| p.strip_prefix("b/").map(str::to_string).unwrap_or(p));
            old_path = None;
            if let Some(p) = &current {
                out.entry(p.clone()).or_default();
            }
        } else if let Some(rest) = line.strip_prefix("--- ") {
            old_path = (rest != "/dev/null").then(|| {
                let p = unquote(rest);
                p.strip_prefix("a/").map(str::to_string).unwrap_or(p)
            });
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            let new_path = (rest != "/dev/null").then(|| {
                let p = unquote(rest);
                p.strip_prefix("b/").map(str::to_string).unwrap_or(p)
            });
            if let Some(p) = new_path.or_else(|| old_path.clone()) {
                if current.as_deref() != Some(p.as_str()) {
                    if let Some(c) = current.take() {
                        let moved = out.remove(&c).unwrap_or_default();
                        out.insert(p.clone(), moved);
                    }
                    current = Some(p);
                }
            }
        } else if line.starts_with("Binary files ") {
            if let Some(entry) = current.as_ref().and_then(|p| out.get_mut(p)) {
                entry.1 = true;
            }
        } else if let Some(rest) = line.strip_prefix("@@ -") {
            let Some(entry) = current.as_ref().and_then(|p| out.get_mut(p)) else {
                continue;
            };
            let range = |s: &str| -> Option<(i64, i64)> {
                let (a, b) = s.split_once(',').unwrap_or((s, "1"));
                Some((a.parse().ok()?, b.parse().ok()?))
            };
            let mut parts = rest.split(' ');
            let old = parts.next().and_then(range);
            let new = parts
                .next()
                .and_then(|s| s.strip_prefix('+'))
                .and_then(range);
            if let (Some(old), Some(new)) = (old, new) {
                entry.0.push(Hunk { old, new });
            }
        }
    }
    out
}

/// Line ranges a list of `(start, len)` covers (empty ones left out).
fn spans(ranges: impl Iterator<Item = (i64, i64)>) -> Vec<(i64, i64)> {
    ranges
        .filter(|(_, n)| *n > 0)
        .map(|(s, n)| (s, s + n - 1))
        .collect()
}

/// The definitions (by index into `defs`) a span touches, innermost: a
/// container kind is dropped when a definition inside it is touched too.
fn touched(defs: &[(i64, i64, &str)], span: (i64, i64)) -> Vec<usize> {
    let hit: Vec<usize> = (0..defs.len())
        .filter(|&i| defs[i].0 <= span.1 && span.0 <= defs[i].1)
        .collect();
    hit.iter()
        .copied()
        .filter(|&i| {
            !CONTAINER_KINDS.contains(&defs[i].2)
                || !hit.iter().any(|&j| {
                    j != i
                        && defs[i].0 <= defs[j].0
                        && defs[j].1 <= defs[i].1
                        && (defs[j].0, defs[j].1) != (defs[i].0, defs[i].1)
                })
        })
        .collect()
}

/// Lines of `span` no definition covers.
fn uncovered(defs: &[(i64, i64, &str)], span: (i64, i64)) -> u64 {
    (span.0..=span.1)
        .filter(|l| !defs.iter().any(|d| d.0 <= *l && *l <= d.1))
        .count() as u64
}

fn key_new(n: &NodeRow) -> (String, String, String, String) {
    (
        n.kind.clone(),
        n.container.clone(),
        n.name.clone(),
        n.modpath.clone(),
    )
}

fn key_old(d: &parse::Def) -> (String, String, String, String) {
    (
        d.kind.clone(),
        d.container.clone(),
        d.name.clone(),
        d.modpath.clone(),
    )
}

/// Map one file's hunks to its definitions: `new` from the index (the file
/// now), `old` parsed from the base's blob.
pub(crate) fn map_symbols(
    path: &str,
    hunks: &[Hunk],
    new: &[NodeRow],
    old: &[parse::Def],
) -> (Vec<ChangedSymbol>, u64) {
    let new_spans = spans(hunks.iter().map(|h| h.new));
    let nd: Vec<(i64, i64, &str)> = new.iter().map(|n| (n.l0, n.l1, n.kind.as_str())).collect();
    let od: Vec<(i64, i64, &str)> = old
        .iter()
        .map(|d| (d.l0 as i64, d.l1 as i64, d.kind.as_str()))
        .collect();
    // The k-th definition of a key on one side matches the k-th on the other.
    let ordinal_new: Vec<usize> = (0..new.len())
        .map(|i| {
            new[..i]
                .iter()
                .filter(|m| key_new(m) == key_new(&new[i]))
                .count()
        })
        .collect();
    let ordinal_old: Vec<usize> = (0..old.len())
        .map(|i| {
            old[..i]
                .iter()
                .filter(|m| key_old(m) == key_old(&old[i]))
                .count()
        })
        .collect();
    let find_old = |i: usize| {
        (0..old.len())
            .find(|&j| key_old(&old[j]) == key_new(&new[i]) && ordinal_old[j] == ordinal_new[i])
    };
    let find_new = |j: usize| {
        (0..new.len())
            .find(|&i| key_new(&new[i]) == key_old(&old[j]) && ordinal_new[i] == ordinal_old[j])
    };
    let mut changed_new: BTreeMap<usize, &'static str> = BTreeMap::new();
    let mut deleted: BTreeSet<usize> = BTreeSet::new();
    let mut outside = 0u64;
    for span in &new_spans {
        outside += uncovered(&nd, *span);
        for i in touched(&nd, *span) {
            let change = if find_old(i).is_some() {
                "edited"
            } else {
                "added"
            };
            changed_new.entry(i).or_insert(change);
        }
    }
    for h in hunks {
        let Some(span) = spans(std::iter::once(h.old)).pop() else {
            continue;
        };
        if h.new.1 == 0 {
            outside += uncovered(&od, span);
        }
        for j in touched(&od, span) {
            match find_new(j) {
                Some(i) => {
                    changed_new.entry(i).or_insert("edited");
                }
                None => {
                    deleted.insert(j);
                }
            }
        }
    }
    let mut out: Vec<ChangedSymbol> = changed_new
        .into_iter()
        .map(|(i, change)| ChangedSymbol {
            change,
            node: Some(new[i].clone()),
            old: None,
            path: path.to_string(),
        })
        .collect();
    out.extend(deleted.into_iter().map(|j| ChangedSymbol {
        change: "deleted",
        node: None,
        old: Some(old[j].clone()),
        path: path.to_string(),
    }));
    (out, outside)
}

/// The base's blobs of `paths` (`rev:path`), `None` for one missing or
/// over the byte cap. Sizes first (`--batch-check`), then the contents.
fn blobs_at(git: &Git, rev: &str, paths: &[&str]) -> Result<Vec<Option<Vec<u8>>>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let names: Vec<String> = paths.iter().map(|p| format!("{rev}:{p}")).collect();
    let input = names.iter().map(|n| format!("{n}\n")).collect::<String>();
    let check = git.run_bytes(&["cat-file", "--batch-check"], input.into_bytes())?;
    let check = String::from_utf8_lossy(&check).into_owned();
    let sizes: Vec<Option<u64>> = check
        .lines()
        .map(|l| {
            let parts: Vec<&str> = l.split(' ').collect();
            match parts[..] {
                [_, "blob", size] => size
                    .parse::<u64>()
                    .ok()
                    .filter(|s| *s <= code_index::MAX_FILE_BYTES),
                _ => None,
            }
        })
        .collect();
    if sizes.len() != paths.len() {
        return Err("git cat-file gave a short answer".into());
    }
    let wanted: Vec<usize> = (0..paths.len()).filter(|&i| sizes[i].is_some()).collect();
    let mut out = vec![None; paths.len()];
    if wanted.is_empty() {
        return Ok(out);
    }
    let input = wanted
        .iter()
        .map(|&i| format!("{}\n", names[i]))
        .collect::<String>();
    let raw = git.run_bytes(&["cat-file", "--batch"], input.into_bytes())?;
    let short = || "git cat-file gave a short answer".to_string();
    let mut at = 0usize;
    for &i in &wanted {
        let nl = at
            + raw
                .get(at..)
                .and_then(|r| r.iter().position(|b| *b == b'\n'))
                .ok_or_else(short)?;
        let header = String::from_utf8_lossy(&raw[at..nl]).into_owned();
        let parts: Vec<&str> = header.split(' ').collect();
        let [_, "blob", size] = parts[..] else {
            at = nl + 1;
            continue;
        };
        let size: usize = size.parse().map_err(|_| short())?;
        let end = nl + 1 + size;
        out[i] = Some(raw.get(nl + 1..end).ok_or_else(short)?.to_vec());
        at = end + 1;
    }
    Ok(out)
}

/// The commit `rev` names, or the default base (the merge-base of HEAD
/// and `origin/HEAD`, else HEAD).
pub(crate) fn resolve_base(git: &Git, rev: &str) -> Result<String, String> {
    if rev.is_empty() {
        return code_index::overlay::base_commit(git)
            .ok_or_else(|| "no such commit: this checkout has no commit yet, so there is no base to diff against.".to_string());
    }
    code_index::overlay::full_commit(git, rev).ok_or_else(|| {
        format!("no such commit `{rev}` here (base takes a git rev: a branch, tag or commit; default the merge-base with origin/HEAD).")
    })
}

/// The files changed from `base` to `head` (a commit), or to the work tree
/// and its untracked, unignored files when `head` is `None`, each mapped
/// to its definitions through `index` (which must be the index of `head`).
pub(crate) fn diff(
    git: &Git,
    index: &Index,
    base: &str,
    head: Option<&str>,
    paths: &[String],
) -> Result<Vec<ChangedFile>, String> {
    let mut range: Vec<&str> = vec![base];
    if let Some(h) = head {
        range.push(h);
    }
    let pathspec: Vec<&str> = paths.iter().map(String::as_str).collect();
    let common = [
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-renames",
        "--no-relative",
    ];
    let mut args: Vec<&str> = vec!["diff", "--name-status", "-z"];
    args.extend(common);
    args.extend(&range);
    args.push("--");
    args.extend(&pathspec);
    let listed = git.run(&args)?;
    let mut entries: Vec<(String, &'static str)> = Vec::new();
    let mut parts = listed.split('\0').filter(|s| !s.is_empty());
    while let (Some(status), Some(path)) = (parts.next(), parts.next()) {
        let status = match status.chars().next() {
            Some('A') => "added",
            Some('D') => "deleted",
            _ => "modified",
        };
        entries.push((path.to_string(), status));
    }
    if head.is_none() {
        let mut args = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
        args.extend(&pathspec);
        for path in git.run(&args)?.split('\0').filter(|s| !s.is_empty()) {
            entries.push((path.to_string(), "added"));
        }
    }
    entries.sort();
    entries.dedup_by(|a, b| a.0 == b.0);
    if entries.len() > MAX_CHANGED_FILES {
        return Err(format!(
            "{} changed files, over the changes cap of {MAX_CHANGED_FILES}; pass paths, or a nearer base.",
            entries.len()
        ));
    }
    // Hunks of the indexed-language files, a chunk of paths per call.
    let code: Vec<&str> = entries
        .iter()
        .filter(|(p, s)| Lang::of(p).is_some() && *s != "added")
        .map(|(p, _)| p.as_str())
        .collect();
    let mut patches: HashMap<String, (Vec<Hunk>, bool)> = HashMap::new();
    for chunk in code.chunks(PATCH_CHUNK) {
        let mut args: Vec<&str> = vec![
            "diff",
            "-U0",
            "--inter-hunk-context=0",
            "--src-prefix=a/",
            "--dst-prefix=b/",
        ];
        args.extend(common);
        args.extend(&range);
        args.push("--");
        args.extend(chunk);
        patches.extend(parse_patch(&git.run(&args)?));
    }
    // The base's blobs of modified and deleted code files.
    let olds: Vec<&str> = entries
        .iter()
        .filter(|(p, s)| {
            *s != "added"
                && Lang::of(p).is_some()
                && code_index::readable_path(p)
                && !p.contains('\n')
        })
        .map(|(p, _)| p.as_str())
        .collect();
    let blobs: HashMap<String, Option<Vec<u8>>> = olds
        .iter()
        .map(|p| p.to_string())
        .zip(blobs_at(git, base, &olds)?)
        .collect();
    let mut out = Vec::new();
    for (path, status) in entries {
        let mut f = ChangedFile {
            test: parse::test_path(&path),
            path: path.clone(),
            status,
            ..Default::default()
        };
        let Some(lang) = Lang::of(&path) else {
            f.unmapped = Some(unmapped_reason(&path));
            out.push(f);
            continue;
        };
        let (hunks, binary) = patches.remove(&path).unwrap_or_default();
        if binary {
            f.unmapped = Some("binary: not traced".into());
            out.push(f);
            continue;
        }
        let now = if status == "deleted" {
            None
        } else {
            index.file(&path)?
        };
        if status != "deleted" && now.is_none() {
            f.unmapped = Some(
                "not in the index (hidden, secret-looking, ignored, over 2 MB or outside the root)"
                    .into(),
            );
            out.push(f);
            continue;
        }
        let new = match &now {
            Some(file) => {
                f.sha = file.sha.clone();
                index.defs_in(&path)?
            }
            None => Vec::new(),
        };
        let old: Vec<parse::Def> = match blobs.get(path.as_str()) {
            Some(Some(bytes)) => parse::parse_file(&path, lang, bytes).defs,
            _ => Vec::new(),
        };
        let (symbols, outside): (Vec<ChangedSymbol>, u64) = match status {
            "added" => (
                new.iter()
                    .map(|n| ChangedSymbol {
                        change: "added",
                        node: Some(n.clone()),
                        old: None,
                        path: path.clone(),
                    })
                    .collect(),
                0,
            ),
            "deleted" => {
                if !blobs.contains_key(path.as_str()) || blobs[path.as_str()].is_none() {
                    f.unmapped = Some("deleted, and its old version could not be read (over 2 MB or a hidden path)".into());
                    out.push(f);
                    continue;
                }
                (
                    old.iter()
                        .map(|d| ChangedSymbol {
                            change: "deleted",
                            node: None,
                            old: Some(d.clone()),
                            path: path.clone(),
                        })
                        .collect(),
                    0,
                )
            }
            _ => map_symbols(&path, &hunks, &new, &old),
        };
        f.symbols = symbols;
        f.outside = outside;
        out.push(f);
    }
    Ok(out)
}

fn list_arg(args: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    let items: Vec<String> = args
        .get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if items.len() > MAX_LIST_ARG {
        return Err(format!("`{key}` must be at most {MAX_LIST_ARG} items."));
    }
    Ok(items)
}

/// `paths`, checked: repo-relative, no `..`, not absolute.
fn paths_arg(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let paths = list_arg(args, "paths")?;
    for p in &paths {
        if p.starts_with('/') || p.split('/').any(|s| s == "..") {
            return Err(format!(
                "`paths` must be repo-relative paths inside the checkout (got `{p}`)."
            ));
        }
    }
    Ok(paths
        .into_iter()
        .map(|p| p.trim_start_matches("./").trim_end_matches('/').to_string())
        .filter(|p| !p.is_empty())
        .collect())
}

/// The diff a call asks for: `(base commit, changed files)`.
fn diff_for(
    root: &Path,
    index: &Index,
    args: &Map<String, Value>,
) -> Result<(String, Vec<ChangedFile>), String> {
    let paths = paths_arg(args)?;
    let git = Git::open(root)?;
    let base = resolve_base(&git, str_arg(args, "base"))?;
    let files = diff(&git, index, &base, None, &paths)?;
    Ok((base, files))
}

fn short(commit: &str) -> &str {
    commit.get(..9).unwrap_or(commit)
}

pub(super) fn changes(
    args: &Map<String, Value>,
    req: &Request,
    index: &Index,
    repo: Value,
    fresh: &Freshness,
    root: &Path,
) -> Result<Vec<Value>, String> {
    let (base, files) = diff_for(root, index, args)?;
    let mut rows: Vec<Value> = Vec::new();
    let (mut mapped, mut symbols) = (0usize, 0usize);
    for f in files.iter().filter(|f| f.unmapped.is_none()) {
        mapped += 1;
        symbols += f.symbols.len();
        let mut v = json!({ "path": f.path, "status": f.status });
        if !f.sha.is_empty() {
            v["fileHash"] = json!(f.sha);
            if fresh.stale(&f.path, &f.sha) {
                v["stale"] = json!(true);
            }
        }
        if f.test {
            v["test"] = json!(true);
        }
        v["symbols"] = Value::Array(
            f.symbols
                .iter()
                .take(SYMBOLS_PER_FILE)
                .map(ChangedSymbol::json)
                .collect(),
        );
        if f.symbols.len() > SYMBOLS_PER_FILE {
            v["symbolsTotal"] = json!(f.symbols.len());
        }
        if f.outside > 0 {
            v["linesOutsideDefinitions"] = json!(f.outside);
        }
        rows.push(v);
    }
    for f in files.iter().filter(|f| f.unmapped.is_some()) {
        rows.push(json!({ "path": f.path, "status": f.status, "unmapped": f.unmapped }));
    }
    let unmapped = files.len() - mapped;
    let render = |r: std::ops::Range<usize>| {
        let page = &rows[r];
        let (un, ma): (Vec<&Value>, Vec<&Value>) =
            page.iter().partition(|v| v.get("unmapped").is_some());
        let un: Vec<Value> = un
            .into_iter()
            .map(|v| json!({ "path": v["path"], "status": v["status"], "reason": v["unmapped"] }))
            .collect();
        json!({
            "tool": "changes", "base": short(&base), "repo": repo,
            "totals": { "files": files.len(), "mapped": mapped, "unmapped": unmapped, "symbols": symbols },
            "files": ma, "unmapped": un,
        })
    };
    paged(req, rows.len(), fresh, &render)
}

// ── Tests that cover a change ───────────────────────────────────────────

/// How sure a listed test is, best first.
const CONFIDENCE: &[&str] = &[
    "direct",
    "transitive",
    "import",
    "same-file",
    "same-module",
    "name-match",
];

fn rank(c: &str) -> usize {
    CONFIDENCE
        .iter()
        .position(|x| *x == c)
        .unwrap_or(CONFIDENCE.len())
}

/// What `affected_tests` reads source text through: the work tree's
/// confined reader, or (the evaluation) a commit's blobs.
pub(crate) type Reader<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

/// The `#[path]` module map and the line cache one walk shares.
struct Rust<'a> {
    index: &'a Index,
    read: Reader<'a>,
    lines: RefCell<HashMap<String, Option<Vec<String>>>>,
    /// A `#[path]`-attached file: (declaring file, the declaration's
    /// module path in the index, the module's name).
    attached: RefCell<Option<HashMap<String, (String, String, String)>>>,
}

impl<'a> Rust<'a> {
    fn new(index: &'a Index, read: Reader<'a>) -> Self {
        Rust {
            index,
            read,
            lines: RefCell::new(HashMap::new()),
            attached: RefCell::new(None),
        }
    }

    fn lines_of(&self, path: &str) -> Option<Vec<String>> {
        if let Some(l) = self.lines.borrow().get(path) {
            return l.clone();
        }
        let l = (self.read)(path).map(|b| {
            String::from_utf8_lossy(&b)
                .lines()
                .map(str::to_string)
                .collect()
        });
        self.lines.borrow_mut().insert(path.to_string(), l.clone());
        l
    }

    /// The attribute lines right above line `l0` (1-based), joined.
    fn attrs_above(&self, path: &str, l0: i64) -> String {
        let Some(lines) = self.lines_of(path) else {
            return String::new();
        };
        let mut out = String::new();
        let mut at = l0 - 2;
        while at >= 0 && l0 - at <= 12 {
            let t = lines.get(at as usize).map(|s| s.trim()).unwrap_or("");
            if t.starts_with("#[") || t.starts_with("//") || t.ends_with(")]") || t.ends_with("\"]")
            {
                out.push_str(t);
                out.push('\n');
                at -= 1;
            } else {
                break;
            }
        }
        out
    }

    /// Whether `n` is a `#[test]` function (not a helper in test code).
    fn is_test_fn(&self, n: &NodeRow) -> bool {
        if n.lang != "rust" || !n.test || n.kind != "fn" {
            return false;
        }
        let a = self.attrs_above(&n.path, n.l0);
        a.contains("#[test]") || a.contains("::test]") || a.contains("::test(")
    }

    fn attached(&self) -> HashMap<String, (String, String, String)> {
        if let Some(m) = self.attached.borrow().as_ref() {
            return m.clone();
        }
        let mut map = HashMap::new();
        for d in self.index.defs_of_kind("mod").unwrap_or_default() {
            if d.lang != "rust" || d.l0 != d.l1 {
                continue;
            }
            let attrs = self.attrs_above(&d.path, d.l0);
            let Some(at) = attrs.find("#[path") else {
                continue;
            };
            let rest = &attrs[at..];
            let Some(q0) = rest.find('"') else { continue };
            let Some(len) = rest[q0 + 1..].find('"') else {
                continue;
            };
            let rel = &rest[q0 + 1..q0 + 1 + len];
            let dir = d.path.rsplit_once('/').map_or("", |(d, _)| d);
            if let Some(child) = crate::code_index::resolve::normalize(&format!("{dir}/{rel}")) {
                map.entry(child)
                    .or_insert((d.path.clone(), d.modpath.clone(), d.name.clone()));
            }
        }
        *self.attached.borrow_mut() = Some(map.clone());
        map
    }

    /// A file's module path as cargo names it, following `#[path]`.
    fn module(&self, path: &str) -> String {
        let map = self.attached();
        let mut chain: Vec<(String, String)> = Vec::new();
        let mut at = path.to_string();
        let mut seen = HashSet::new();
        while let Some((parent, decl_mod, name)) = map.get(&at) {
            if !seen.insert(at.clone()) || chain.len() > 16 {
                break;
            }
            let inline = decl_mod
                .strip_prefix(&parse::module_of(parent))
                .unwrap_or("")
                .trim_start_matches("::")
                .to_string();
            chain.push((inline, name.clone()));
            at = parent.clone();
        }
        let mut parts: Vec<String> = vec![parse::module_of(&at)];
        for (inline, name) in chain.into_iter().rev() {
            parts.push(inline);
            parts.push(name);
        }
        parts.retain(|p| !p.is_empty());
        parts.join("::")
    }

    /// A definition's module as cargo names it.
    fn def_module(&self, n: &NodeRow) -> String {
        let inline = n
            .modpath
            .strip_prefix(&parse::module_of(&n.path))
            .unwrap_or("")
            .trim_start_matches("::");
        [self.module(&n.path), inline.to_string()]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("::")
    }

    /// The crate folder a Rust file builds in (before its last `src/`).
    fn crate_dir(path: &str) -> String {
        path.rfind("src/")
            .map_or("", |at| &path[..at])
            .trim_end_matches('/')
            .to_string()
    }
}

/// One test reached, before grouping.
#[derive(Debug, Clone)]
struct Found {
    confidence: &'static str,
    why: String,
}

/// What a walk found: Rust tests by (crate folder, cargo path), module
/// filters by (crate folder, module), test files, and what it can't map.
#[derive(Debug, Default)]
pub(crate) struct Affected {
    rust: BTreeMap<(String, String), Found>,
    modules: BTreeMap<(String, String), Found>,
    files: BTreeMap<String, Found>,
    cannot: Vec<Value>,
}

impl Affected {
    fn keep(map_entry: &mut Found, new: Found) {
        if rank(new.confidence) < rank(map_entry.confidence) {
            *map_entry = new;
        }
    }

    fn rust_test(&mut self, key: (String, String), f: Found) {
        match self.rust.get_mut(&key) {
            Some(e) => Self::keep(e, f),
            None => {
                self.rust.insert(key, f);
            }
        }
    }

    fn module(&mut self, key: (String, String), f: Found) {
        match self.modules.get_mut(&key) {
            Some(e) => Self::keep(e, f),
            None => {
                self.modules.insert(key, f);
            }
        }
    }

    fn file(&mut self, path: String, f: Found) {
        match self.files.get_mut(&path) {
            Some(e) => Self::keep(e, f),
            None => {
                self.files.insert(path, f);
            }
        }
    }

    fn cannot(&mut self, v: Value) {
        if !self.cannot.contains(&v) {
            self.cannot.push(v);
        }
    }

    /// Rust filters (a module's tests grouped under its narrowest filter)
    /// and test files, best confidence first.
    pub(crate) fn rows(&self) -> Vec<Value> {
        let mut groups: BTreeMap<(String, String), Vec<(String, &Found)>> = BTreeMap::new();
        for ((dir, path), f) in &self.rust {
            let (module, _) = path.rsplit_once("::").unwrap_or(("", path));
            groups
                .entry((dir.clone(), module.to_string()))
                .or_default()
                .push((path.clone(), f));
        }
        let mut rows: Vec<(usize, String, Value)> = Vec::new();
        let command = |dir: &str, filter: &str| {
            if dir.is_empty() {
                format!("cargo test --lib {filter}")
            } else {
                format!("cd {dir} && cargo test --lib {filter}")
            }
        };
        let mut module_rows: BTreeSet<(String, String)> = BTreeSet::new();
        for ((dir, module), tests) in &groups {
            let whole = self.modules.get(&(dir.clone(), module.clone()));
            let filter = if tests.len() == 1 && whole.is_none() {
                tests[0].0.clone()
            } else {
                format!("{module}::")
            };
            if whole.is_some() {
                module_rows.insert((dir.clone(), module.clone()));
            }
            let mut best = tests
                .iter()
                .map(|t| t.1.confidence)
                .min_by_key(|c| rank(c))
                .unwrap_or("same-module");
            if let Some(w) = whole {
                if rank(w.confidence) < rank(best) {
                    best = w.confidence;
                }
            }
            let mut sorted = tests.clone();
            sorted.sort_by(|a, b| {
                rank(a.1.confidence)
                    .cmp(&rank(b.1.confidence))
                    .then(a.0.cmp(&b.0))
            });
            let listed: Vec<Value> = sorted
                .iter()
                .take(TESTS_PER_GROUP)
                .map(|(p, f)| json!({ "name": p, "confidence": f.confidence, "why": f.why }))
                .collect();
            let mut v = json!({ "runner": "cargo", "command": command(dir, &filter), "filter": filter, "confidence": best, "tests": listed });
            if tests.len() > TESTS_PER_GROUP {
                v["testsTotal"] = json!(tests.len());
            }
            if let Some(w) = whole {
                v["why"] = json!(w.why);
                v["wholeModule"] = json!(true);
            }
            rows.push((rank(best), command(dir, &filter), v));
        }
        for ((dir, module), f) in &self.modules {
            if module_rows.contains(&(dir.clone(), module.clone()))
                || groups.contains_key(&(dir.clone(), module.clone()))
            {
                continue;
            }
            let filter = format!("{module}::");
            let v = json!({ "runner": "cargo", "command": command(dir, &filter), "filter": filter, "confidence": f.confidence, "wholeModule": true, "why": f.why });
            rows.push((rank(f.confidence), command(dir, &filter), v));
        }
        for (path, f) in &self.files {
            // By convention: `.spec.` files are Playwright's, `.test.` vitest's.
            let file = path.rsplit('/').next().unwrap_or(path);
            let (runner, cmd) = if path.ends_with(".py") {
                ("pytest", format!("python -m pytest {path}"))
            } else if file.contains(".spec.") {
                ("playwright", format!("npx playwright test {path}"))
            } else {
                ("vitest", format!("npx vitest run {path}"))
            };
            let v = json!({ "runner": runner, "command": cmd, "path": path, "confidence": f.confidence, "why": f.why });
            rows.push((rank(f.confidence), cmd, v));
        }
        rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        rows.into_iter().map(|r| r.2).collect()
    }

    /// The best confidence a listed test (a cargo path or a test file)
    /// was found with, for the evaluation.
    #[cfg(test)]
    pub(crate) fn confidence_for(&self, test: &str) -> Option<&'static str> {
        let mut best: Option<&'static str> = self.files.get(test).map(|f| f.confidence);
        let mut take = |c: &'static str| {
            if best.is_none_or(|b| rank(c) < rank(b)) {
                best = Some(c);
            }
        };
        for ((_, path), f) in &self.rust {
            if path == test {
                take(f.confidence);
            }
        }
        for ((_, module), f) in &self.modules {
            if test.starts_with(&format!("{module}::")) {
                take(f.confidence);
            }
        }
        best
    }

    /// The Rust filters and test files listed, for the evaluation.
    #[cfg(test)]
    pub(crate) fn listed(&self) -> (Vec<String>, Vec<String>) {
        let rows = self.rows();
        let filters = rows
            .iter()
            .filter_map(|r| r["filter"].as_str().map(str::to_string))
            .collect();
        let files = rows
            .iter()
            .filter_map(|r| r["path"].as_str().map(str::to_string))
            .collect();
        (filters, files)
    }

    pub(crate) fn cannot_map(&self) -> &[Value] {
        &self.cannot
    }
}

/// One function the walk reached: the changed symbol it came from (an
/// index into the roots), its parent in the walk and its level.
struct Reach {
    node: NodeRow,
    root: usize,
    parent: Option<usize>,
    level: u64,
    weak: bool,
}

fn chain(reached: &[Reach], mut at: usize, roots: &[ChangedSymbol]) -> String {
    let mut names = Vec::new();
    loop {
        let r = &reached[at];
        names.push(r.node.name.clone());
        match r.parent {
            Some(p) => at = p,
            None => break,
        }
    }
    names.reverse();
    let root = &roots[reached[at].root];
    let head = format!(
        "{} {} ({}:{})",
        root.change, names[0], root.path, reached[at].node.l0
    );
    std::iter::once(head)
        .chain(names.into_iter().skip(1))
        .collect::<Vec<_>>()
        .join(" <- ")
}

/// Walk from `roots` (changed definitions) and `files` (the changed files,
/// for the same-file, same-module and import rules) to the tests that
/// cover them.
/// The rules a walk uses beyond resolved calls; [`Tuning::TOOL`] is what
/// the tool runs (the evaluation compares others).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tuning {
    /// Follow a rare name's unresolved calls in TypeScript and Python too
    /// (Rust always).
    pub name_match_ts: bool,
    /// Levels of the TypeScript import walk (at most `depth`).
    pub import_depth: u64,
}

impl Tuning {
    /// Measured on 10 merged PRs (docs/CODE_CONTEXT.md): the leanest of
    /// the four compared (432 rows against 725 with both on), with no test
    /// missed.
    pub(crate) const TOOL: Tuning = Tuning {
        name_match_ts: false,
        import_depth: 1,
    };
}

pub(crate) fn affected(
    index: &Index,
    read: Reader,
    roots: &[ChangedSymbol],
    files: &[ChangedFile],
    depth: u64,
    tuning: Tuning,
) -> Result<Affected, String> {
    let rust = Rust::new(index, read);
    let mut out = Affected::default();
    let mut reached: Vec<Reach> = Vec::new();
    let mut seen: HashMap<i64, usize> = HashMap::new();
    let mut queue: VecDeque<usize> = VecDeque::new();
    let mut past_depth = 0usize;
    let mut capped = false;
    for (i, s) in roots.iter().enumerate() {
        match &s.node {
            Some(n) => {
                if seen.contains_key(&n.id) {
                    continue;
                }
                seen.insert(n.id, reached.len());
                queue.push_back(reached.len());
                reached.push(Reach {
                    node: n.clone(),
                    root: i,
                    parent: None,
                    level: 0,
                    weak: false,
                });
                if n.command {
                    out.cannot(json!({ "symbol": n.name, "path": n.path, "reason": "Tauri command: the webview calls it by string (invoke); only resolved invoke calls are followed, and TypeScript tests mock invoke" }));
                }
            }
            None => {
                let hits = index.edges_named(s.name()).map(|e| e.len()).unwrap_or(0);
                out.cannot(json!({ "symbol": s.name(), "path": s.path, "reason": format!("deleted: its remaining uses ({hits} name-scan hits) no longer build or resolve; the build and the full gate catch them") }));
            }
        }
    }
    let defs_count = |name: &str, lang: &str| -> usize {
        index
            .defs_named(name)
            .map(|d| d.iter().filter(|n| family(&n.lang) == family(lang)).count())
            .unwrap_or(0)
    };
    while let Some(i) = queue.pop_front() {
        let (node, level, weak) = (reached[i].node.clone(), reached[i].level, reached[i].weak);
        let mut edges: Vec<(EdgeRow, bool)> = index
            .edges_to(node.id)?
            .into_iter()
            .map(|e| (e, weak))
            .collect();
        // Unresolved calls of a rare name, and what they can't follow.
        let named = index.edges_named(&node.name)?;
        let unresolved: Vec<EdgeRow> = named
            .into_iter()
            .filter(|e| e.target.is_none() && family(&e.lang) == family(&node.lang))
            .collect();
        let macro_hits = unresolved
            .iter()
            .filter(|e| e.how.starts_with("macro-args"))
            .count();
        let method_hits = unresolved
            .iter()
            .filter(|e| e.how.starts_with("ambiguous-method"))
            .count();
        let rare = defs_count(&node.name, &node.lang) <= NAME_MATCH_MAX_DEFS;
        if level == 0 && macro_hits > 0 {
            out.cannot(json!({ "symbol": node.name, "path": node.path, "reason": format!("{macro_hits} references inside macro arguments are not resolved to it; tests reaching it only that way are not listed") }));
        }
        if level == 0 && method_hits > 0 && !rare {
            out.cannot(json!({ "symbol": node.name, "path": node.path, "reason": format!("{method_hits} method calls on receivers of unknown type (trait objects, generics, several types with this method) are not followed") }));
        }
        if rare && (tuning.name_match_ts || node.lang == "rust") {
            edges.extend(
                unresolved
                    .into_iter()
                    .filter(|e| {
                        matches!(e.kind.as_str(), "call" | "method")
                            && !e.how.starts_with("macro-args")
                    })
                    .map(|e| (e, true)),
            );
        }
        for (e, weak) in edges {
            let conf = |via_import: bool| -> &'static str {
                if weak {
                    "name-match"
                } else if via_import {
                    "import"
                } else if level == 0 {
                    "direct"
                } else {
                    "transitive"
                }
            };
            if e.test() {
                let why = |last: &str| {
                    let c = chain(&reached, i, roots);
                    if last.is_empty() {
                        c
                    } else {
                        format!("{c} <- {last}")
                    }
                };
                match (&e.src, e.lang.as_str()) {
                    (Some(src), "rust") if rust.is_test_fn(src) => {
                        let path = format!("{}::{}", rust.def_module(src), src.name);
                        out.rust_test(
                            (Rust::crate_dir(&src.path), path),
                            Found {
                                confidence: conf(false),
                                why: why(&src.name),
                            },
                        );
                    }
                    (Some(src), "rust") => {
                        // A helper in test code: its callers, at the same level.
                        if !seen.contains_key(&src.id) && reached.len() < REACH_CAP {
                            seen.insert(src.id, reached.len());
                            queue.push_front(reached.len());
                            reached.push(Reach {
                                node: src.clone(),
                                root: reached[i].root,
                                parent: Some(i),
                                level,
                                weak,
                            });
                        }
                    }
                    (None, "rust") => {
                        let module = rust.module(&e.path);
                        out.module(
                            (Rust::crate_dir(&e.path), module),
                            Found {
                                confidence: conf(false),
                                why: why(""),
                            },
                        );
                    }
                    _ => {
                        let last = e.src.as_ref().map(|s| s.name.clone()).unwrap_or_default();
                        out.file(
                            e.path.clone(),
                            Found {
                                confidence: conf(e.kind == "import"),
                                why: why(&last),
                            },
                        );
                    }
                }
                continue;
            }
            let Some(src) = e.src else { continue };
            if e.kind == "import" || seen.contains_key(&src.id) {
                continue;
            }
            if level + 1 > depth {
                past_depth += 1;
                continue;
            }
            if reached.len() >= REACH_CAP {
                capped = true;
                continue;
            }
            seen.insert(src.id, reached.len());
            queue.push_back(reached.len());
            reached.push(Reach {
                node: src,
                root: reached[i].root,
                parent: Some(i),
                level: level + 1,
                weak,
            });
        }
    }
    if past_depth > 0 {
        out.cannot(json!({ "reason": format!("{past_depth} callers past depth {depth} not followed (raise depth, up to {AFFECTED_MAX_DEPTH})") }));
    }
    if capped {
        out.cannot(json!({ "reason": format!("the walk stopped at {REACH_CAP} functions; callers past them are not followed (pass fewer symbols or a smaller depth)") }));
    }
    same_file_rules(&rust, index, roots, files, &mut out)?;
    import_rule(index, files, depth.min(tuning.import_depth), &mut out)?;
    for f in files {
        if let Some(why) = &f.unmapped {
            out.cannot(json!({ "path": f.path, "reason": why }));
        } else if f.outside > 0 && Lang::of(&f.path) == Some(Lang::Rust) {
            out.cannot(json!({ "path": f.path, "reason": format!("{} changed lines outside any definition (use, attributes, top-level items); its file's test modules are listed instead", f.outside) }));
        }
    }
    Ok(out)
}

/// Rust: every test module in a changed file (`same-file`) or attached to
/// it with `#[path]` (`same-module`), as one module filter each.
fn same_file_rules(
    rust: &Rust,
    index: &Index,
    roots: &[ChangedSymbol],
    files: &[ChangedFile],
    out: &mut Affected,
) -> Result<(), String> {
    let mut changed: BTreeMap<String, String> = BTreeMap::new();
    for s in roots {
        if Lang::of(&s.path) == Some(Lang::Rust) {
            changed
                .entry(s.path.clone())
                .or_insert_with(|| format!("{} {}", s.change, s.name()));
        }
    }
    for f in files
        .iter()
        .filter(|f| f.unmapped.is_none() && f.status != "deleted")
    {
        if Lang::of(&f.path) == Some(Lang::Rust) && (f.outside > 0 || !f.symbols.is_empty()) {
            changed
                .entry(f.path.clone())
                .or_insert_with(|| "changed lines".into());
        }
    }
    if changed.is_empty() {
        return Ok(());
    }
    let attached = rust.attached();
    for (path, what) in &changed {
        let dir = Rust::crate_dir(path);
        let why = format!("{what} in {path}");
        // Inline `#[cfg(test)] mod` blocks of the file itself.
        for d in index.defs_in(path)? {
            if d.kind == "mod" && d.test && d.l0 != d.l1 {
                let module = format!("{}::{}", rust.def_module(&d), d.name)
                    .trim_start_matches("::")
                    .to_string();
                out.module(
                    (dir.clone(), module),
                    Found {
                        confidence: "same-file",
                        why: why.clone(),
                    },
                );
            }
        }
        // `#[path]` test files declared in it.
        for (child, (parent, _, _)) in &attached {
            if parent == path && parse::test_path(child) && index.file(child)?.is_some() {
                out.module(
                    (dir.clone(), rust.module(child)),
                    Found {
                        confidence: "same-module",
                        why: why.clone(),
                    },
                );
            }
        }
    }
    Ok(())
}

/// TypeScript and Python: test files importing a changed file, then files
/// importing those, to `depth` levels.
fn import_rule(
    index: &Index,
    files: &[ChangedFile],
    depth: u64,
    out: &mut Affected,
) -> Result<(), String> {
    let mut frontier: Vec<(String, String)> = files
        .iter()
        .filter(|f| f.unmapped.is_none() && f.status != "deleted" && !f.test)
        .filter(|f| matches!(Lang::of(&f.path), Some(Lang::Ts | Lang::Tsx | Lang::Python)))
        .map(|f| (f.path.clone(), f.path.clone()))
        .collect();
    let mut seen: HashSet<String> = frontier.iter().map(|f| f.0.clone()).collect();
    for level in 1..=depth.max(1) {
        let mut next = Vec::new();
        for (path, why) in &frontier {
            for d in index.defs_in(path)? {
                for e in index.edges_to(d.id)? {
                    if e.kind != "import" || e.path == *path {
                        continue;
                    }
                    let chain = format!("{why} <- imported by {}", e.path);
                    if e.test() {
                        out.file(
                            e.path.clone(),
                            Found {
                                confidence: "import",
                                why: chain,
                            },
                        );
                    } else if level < depth
                        && seen.len() < IMPORT_FILES_CAP
                        && seen.insert(e.path.clone())
                    {
                        next.push((e.path.clone(), chain));
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    Ok(())
}

/// `symbols` resolved to definitions (`name` or `Qualifier::name`).
fn named_roots(
    index: &Index,
    names: &[String],
) -> Result<(Vec<ChangedSymbol>, Vec<Value>), String> {
    let mut roots = Vec::new();
    let mut missing = Vec::new();
    for raw in names {
        let (qual, name) = match raw.rsplit_once("::") {
            Some((q, n)) => (q.rsplit("::").next().unwrap_or(q), n),
            None => ("", raw.as_str()),
        };
        let mut a = Map::new();
        a.insert("name".into(), json!(name));
        a.insert("qualifier".into(), json!(qual));
        let defs = targets(index, &a)?;
        if defs.is_empty() {
            missing.push(json!({ "symbol": raw, "reason": not_found(index, name) }));
        }
        roots.extend(defs.into_iter().map(|d| ChangedSymbol {
            change: "named",
            path: d.path.clone(),
            node: Some(d),
            old: None,
        }));
    }
    Ok((roots, missing))
}

pub(super) fn affected_tests(
    args: &Map<String, Value>,
    req: &Request,
    index: &Index,
    repo: Value,
    fresh: &Freshness,
    root: &Path,
) -> Result<Vec<Value>, String> {
    let depth = args.get("depth").and_then(Value::as_u64).unwrap_or(2);
    let names = list_arg(args, "symbols")?;
    if !names.is_empty()
        && (!str_arg(args, "base").is_empty() || args.get("paths").is_some_and(|p| !p.is_null()))
    {
        return Err(
            "`symbols` must be given alone: pass symbols, or base and paths, not both.".into(),
        );
    }
    let canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let read = |p: &str| code_index::read_index_path(&canon, p);
    let (from, base, roots, files, missing) = if names.is_empty() {
        let (base, files) = diff_for(root, index, args)?;
        let roots: Vec<ChangedSymbol> = files.iter().flat_map(|f| f.symbols.clone()).collect();
        let from = json!({ "changedFiles": files.len(), "changedSymbols": roots.len() });
        (from, Some(base), roots, files, Vec::new())
    } else {
        let (roots, missing) = named_roots(index, &names)?;
        (
            json!({ "symbols": names, "definitions": roots.len() }),
            None,
            roots,
            Vec::new(),
            missing,
        )
    };
    let mut found = affected(index, &read, &roots, &files, depth, Tuning::TOOL)?;
    for m in missing {
        found.cannot(m);
    }
    let rows = found.rows();
    let cannot = found.cannot_map().to_vec();
    let rust_filters = rows.iter().filter(|r| r["runner"] == "cargo").count();
    let run = combined(&rows);
    let render = |r: std::ops::Range<usize>| {
        let mut out =
            json!({ "tool": "affected_tests", "repo": repo, "from": from, "depth": depth });
        if let Some(b) = &base {
            out["base"] = json!(short(b));
        }
        out["totals"] = json!({ "rustFilters": rust_filters, "testFiles": rows.len() - rust_filters, "cannotMap": cannot.len() });
        if r.start == 0 {
            out["run"] = run.clone();
        }
        out["tests"] = Value::Array(rows[r].to_vec());
        out["cannotMap"] = Value::Array(cannot.iter().take(CANNOT_MAP_SHOWN).cloned().collect());
        out["missRate"] = json!(MISS_RATE);
        out["fullGateStillRequired"] = json!(true);
        out["note"] = json!("A lean local check, never complete: run these first, then the full gate. Commands run from the repo root.");
        out
    };
    paged(req, rows.len(), fresh, &render)
}

/// One command per crate folder (cargo takes several filters after `--`)
/// and one vitest command, when they fit in [`RUN_BYTES`]; else a note.
fn combined(rows: &[Value]) -> Value {
    let mut cargo: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut vitest: Vec<&str> = Vec::new();
    let mut playwright: Vec<&str> = Vec::new();
    for r in rows {
        let command = r["command"].as_str().unwrap_or("");
        match (
            r["runner"].as_str(),
            r["filter"].as_str(),
            r["path"].as_str(),
        ) {
            (Some("cargo"), Some(filter), _) => {
                let dir = command
                    .strip_prefix("cd ")
                    .and_then(|c| c.split_once(" && "))
                    .map_or("", |(d, _)| d);
                cargo.entry(dir.to_string()).or_default().push(filter);
            }
            (Some("vitest"), _, Some(path)) => vitest.push(path),
            (Some("playwright"), _, Some(path)) => playwright.push(path),
            _ => {}
        }
    }
    let mut out: Vec<String> = cargo
        .into_iter()
        .map(|(dir, filters)| {
            let run = format!("cargo test --lib -- {}", filters.join(" "));
            if dir.is_empty() {
                run
            } else {
                format!("cd {dir} && {run}")
            }
        })
        .collect();
    if !vitest.is_empty() {
        out.push(format!("npx vitest run {}", vitest.join(" ")));
    }
    if !playwright.is_empty() {
        out.push(format!("npx playwright test {}", playwright.join(" ")));
    }
    if out.iter().map(String::len).sum::<usize>() > RUN_BYTES {
        return json!(format!(
            "{} rows: too many to join into one command here; run them from tests, or narrow paths.",
            rows.len()
        ));
    }
    json!(out)
}

/// `run` joins the listed commands only up to this many bytes.
const RUN_BYTES: usize = 3_000;

#[cfg(test)]
#[path = "code_context_changes_tests.rs"]
mod tests;
