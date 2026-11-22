//! CC3 (design 2.7): worktree overlays over a base index shared by every
//! worktree of a repository.
//!
//! **Base**: the index of one commit, built from that commit's blobs
//! (`git ls-tree` and `git cat-file --batch` through the hardened [`Git`],
//! never the work tree), so it is a pure function of the commit. One file
//! per repository and commit (`base-<repo>-<commit>.sqlite`, the repo keyed
//! on its canonical git common dir) in the index folder, immutable once
//! swapped in, read by every worktree; at most [`BASE_KEEP`] generations
//! per repository (the least recently used are pruned). The commit is the
//! merge-base of HEAD and `origin/HEAD` (HEAD without one); when that base
//! is missing, a kept generation whose diff from the work tree is small is
//! used instead, so a fresh worktree answers without a build.
//!
//! **Overlay**: per worktree, rows only for files whose content differs
//! from the base (`git diff --name-only <base>`, then the 12-hex SHA-256
//! compared) and for untracked, unignored files; a deleted (or now
//! unreadable) file is a tombstone in `hidden`. Its node ids continue after
//! the base's. Its edges resolve against the merged definitions; base edges
//! are re-resolved, from their stored rows (a name lookup, no re-parse),
//! only for names defined in changed files (or whose relative import now
//! names another file), and the ones whose annotation changed are stored
//! as `edge_fix` rows. Its fingerprint is CC2's over the merged files, so a
//! cursor binds to base plus overlay.
//!
//! A checkout with no commit, or whose overlay would be most of the
//! repository, reads CC2's full index ([`super::ensure_with`]) instead.
//!
//! The Mac mini builds a base with `hollow code-context build-base` and
//! this Mac takes it in with `hollow code-context import-base`
//! ([`cli_build_base`], [`cli_import_base`]; docs/CODE_CONTEXT.md).

use super::{
    abort_text, build_locked, cache_dir, ensure_with, hex, list_with, parse, parse_with,
    read_inside, readable_path, resolve, short_sha, stamp, store, within_limits, BuildLock,
    FileParse, Git, Limits, Listed, Listing, BUILD_SEQ, LIMITS, LIMIT_REFUSAL, MAX_FILE_BYTES,
};
use parse::Lang;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// How an overlay with too many changed files is refused.
const OVERLAY_UNUSABLE: &str = "overlay unusable:";

/// Base generations kept per repository.
pub(crate) const BASE_KEEP: usize = 3;
/// A kept base whose diff from the work tree is at most this many files
/// stands in for a missing merge-base one.
const REUSE_MAX_CHANGED: usize = 500;
/// An overlay over this many files that is also over half its base does
/// not pay: the worktree reads the full index.
const OVERLAY_MAX_FILES: usize = 2000;
/// `build-base` on the command line is offline work: a longer deadline.
const CLI_DEADLINE: Duration = Duration::from_secs(900);
/// A base used or written this recently is never pruned (another worktree
/// may be about to read it).
const PRUNE_GRACE: Duration = Duration::from_secs(30 * 60);
/// The largest base file `import-base` takes (this repository's is about
/// 114 MB; the full-index limits allow some 20 times that).
const MAX_IMPORT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Overlay files last found unusable (too many changed files), with the
/// stamp they were found so under and the reason: the same stamp gives the
/// same verdict without re-hashing the work tree.
static UNUSABLE: once_cell::sync::Lazy<Mutex<HashMap<PathBuf, (String, String)>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

/// The index a code-context call reads for `root`: base plus overlay when
/// the checkout has a commit, else (or when that can't be had) CC2's full
/// index.
pub(crate) fn open(root: &Path, cache: Option<&Path>) -> Result<store::Index, String> {
    open_with(root, cache, &LIMITS)
}

/// [`open`] under `limits`.
pub(crate) fn open_with(
    root: &Path,
    cache: Option<&Path>,
    limits: &Limits,
) -> Result<store::Index, String> {
    let deadline = Instant::now() + limits.deadline;
    let git = Git::open(root)?;
    if let Some(want) = base_commit(&git) {
        match overlaid(&git, cache, &want, limits, deadline) {
            Ok(index) => return Ok(index),
            Err(e) if e.starts_with(LIMIT_REFUSAL) => return Err(e),
            Err(_) => {} // no usable base: the full index below
        }
    }
    let path = ensure_with(root, cache, limits)?;
    store::Index::open(&path)
}

fn overlaid(
    git: &Git,
    cache: Option<&Path>,
    want: &str,
    limits: &Limits,
    deadline: Instant,
) -> Result<store::Index, String> {
    let dir = cache_dir(cache).ok_or("no cache folder for the code index")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create the index folder: {e}"))?;
    let key = repo_key(git)?;
    let listing = list_with(git)?;
    within_limits(&listing, limits)?;
    let mut commit = pick_base(git, &dir, &key, want);
    let base = if base_ok(&base_path(&dir, &key, &commit), &commit) {
        base_path(&dir, &key, &commit)
    } else {
        // One base build per repository at a time: two worktrees wanting
        // two commits wait for each other, and the second then takes the
        // first's base when its diff is small instead of building another.
        let _repo = BuildLock::acquire(dir.join(format!(".base-{key}.lock")), deadline)?;
        commit = pick_base(git, &dir, &key, want);
        ensure_base(git, &dir, &key, &commit, limits, deadline)?
    };
    // A base swapped between the overlay check and the attach (a prune
    // and rebuild, an import) is caught on attach; the overlay is then
    // rebuilt on the base now there, once.
    let mut tries = 0;
    loop {
        let overlay = ensure_overlay(git, &listing, &dir, &base, &commit, limits, deadline)?;
        match store::Index::open_overlay(&base, &overlay) {
            Err(e) if e == store::OVERLAY_MISMATCH && tries == 0 => tries += 1,
            done => {
                touch(&base);
                return done;
            }
        }
    }
}

/// A full commit id (40 or 64 lowercase hex digits).
pub(crate) fn is_sha(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The full id of commit `rev` in this repository, if there is one.
pub(crate) fn full_commit(git: &Git, rev: &str) -> Option<String> {
    if rev.is_empty() || rev.starts_with('-') {
        return None;
    }
    let out = git
        .run(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ])
        .ok()?;
    let sha = out.trim();
    is_sha(sha).then(|| sha.to_string())
}

/// The commit a worktree's base is: the merge-base of HEAD and
/// `origin/HEAD`, else HEAD; `None` before the first commit.
pub(crate) fn base_commit(git: &Git) -> Option<String> {
    let head = full_commit(git, "HEAD")?;
    full_commit(git, "refs/remotes/origin/HEAD")
        .and_then(|upstream| git.run(&["merge-base", &head, &upstream]).ok())
        .map(|m| m.trim().to_string())
        .filter(|m| is_sha(m))
        .or(Some(head))
}

/// The repository's key: a hash of its canonical git common dir, the same
/// for every worktree of it.
pub(crate) fn repo_key(git: &Git) -> Result<String, String> {
    let out = git.run(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let dir = std::fs::canonicalize(out.trim())
        .map_err(|e| format!("git named common dir `{}`: {e}", out.trim()))?;
    Ok(short_sha(dir.to_string_lossy().as_bytes()))
}

/// The base file of `commit` for repository `key` under `dir`.
pub(crate) fn base_path(dir: &Path, key: &str, commit: &str) -> PathBuf {
    dir.join(format!("base-{key}-{commit}.sqlite"))
}

/// A whole base of `commit` this Hollow can read.
fn base_ok(path: &Path, commit: &str) -> bool {
    let Ok(index) = store::Index::open(path) else {
        return false;
    };
    index.meta("kind").as_deref() == Some("base")
        && index.meta("base").as_deref() == Some(commit)
        && index.meta("schema") == Some(store::SCHEMA_VERSION.to_string())
        && index.meta("parsers").as_deref() == Some(parse::PARSERS)
}

/// Repository `key`'s base files under `dir`: `(commit, path)`, the most
/// recently used first.
fn generations(dir: &Path, key: &str) -> Vec<(String, PathBuf)> {
    let prefix = format!("base-{key}-");
    let mut out: Vec<(SystemTime, String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let commit = name.strip_prefix(&prefix)?.strip_suffix(".sqlite")?;
            if !is_sha(commit) {
                return None;
            }
            let used = entry.metadata().and_then(|m| m.modified()).ok()?;
            Some((used, commit.to_string(), entry.path()))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    out.into_iter().map(|(_, c, p)| (c, p)).collect()
}

/// `want` when its base exists; else the most recently used kept base
/// whose diff from the work tree is small; else `want` (to be built).
fn pick_base(git: &Git, dir: &Path, key: &str, want: &str) -> String {
    if base_ok(&base_path(dir, key, want), want) {
        return want.to_string();
    }
    for (commit, path) in generations(dir, key) {
        if commit == want || !base_ok(&path, &commit) || full_commit(git, &commit).is_none() {
            continue;
        }
        if changed_paths(git, &commit).is_ok_and(|p| p.len() <= REUSE_MAX_CHANGED) {
            return commit;
        }
    }
    want.to_string()
}

/// Mark a base used now (generations are pruned least recently used
/// first). Best effort.
fn touch(path: &Path) {
    if let Ok(f) = std::fs::File::options().write(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Keep `keep` and the [`BASE_KEEP`] - 1 most recently used other bases of
/// repository `key`; remove the rest (an open reader keeps its file),
/// except any used or written in the last [`PRUNE_GRACE`].
fn prune_bases(dir: &Path, key: &str, keep: &Path) {
    let others = generations(dir, key)
        .into_iter()
        .filter(|(_, p)| p != keep)
        .skip(BASE_KEEP.saturating_sub(1));
    let now = SystemTime::now();
    for (_, path) in others {
        let recent = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_none_or(|age| age < PRUNE_GRACE);
        if !recent {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The base of `commit`, built (under the build lock) when missing.
fn ensure_base(
    git: &Git,
    dir: &Path,
    key: &str,
    commit: &str,
    limits: &Limits,
    deadline: Instant,
) -> Result<PathBuf, String> {
    let path = base_path(dir, key, commit);
    let current = || base_ok(&path, commit);
    if !current() {
        build_locked(&path, deadline, &current, |tmp| {
            build_base(git, commit, tmp, limits, deadline).map(|_| ())
        })?;
        prune_bases(dir, key, &path);
    }
    Ok(path)
}

/// Paths whose content in the work tree may differ from `commit`'s
/// (tracked files only; renames as a delete and an add): git's diff, plus
/// every assume-unchanged or skip-worktree file (`git ls-files -v` tags
/// lowercase or `S`), whose in-place edits that diff does not see.
fn changed_paths(git: &Git, commit: &str) -> Result<Vec<String>, String> {
    let out = git.run(&[
        "diff",
        "--name-only",
        "-z",
        "--no-renames",
        "--ignore-submodules=all",
        commit,
        "--",
    ])?;
    let mut paths: BTreeSet<String> = out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    let tagged = git.run(&["ls-files", "-v", "-z", "--full-name"])?;
    paths.extend(tagged.split('\0').filter_map(|e| {
        let (tag, path) = e.split_once(' ')?;
        let hidden = tag.chars().all(|c| c.is_ascii_lowercase()) || tag == "S";
        (hidden && !tag.is_empty() && !path.is_empty()).then(|| path.to_string())
    }));
    Ok(paths.into_iter().collect())
}

/// The bytes of blobs `oids`, in order (`git cat-file --batch`: no filter
/// or textconv runs).
fn read_blobs(git: &Git, oids: &[&str]) -> Result<Vec<Vec<u8>>, String> {
    if oids.is_empty() {
        return Ok(Vec::new());
    }
    let mut input = String::with_capacity(oids.len() * 41);
    for oid in oids {
        input.push_str(oid);
        input.push('\n');
    }
    let out = git.run_bytes(&["cat-file", "--batch"], input.into_bytes())?;
    let short = || "git cat-file gave a short answer".to_string();
    let mut blobs = Vec::with_capacity(oids.len());
    let mut at = 0usize;
    for oid in oids {
        let nl = at
            + out
                .get(at..)
                .and_then(|rest| rest.iter().position(|b| *b == b'\n'))
                .ok_or_else(short)?;
        let header = String::from_utf8_lossy(&out[at..nl]);
        let parts: Vec<&str> = header.split(' ').collect();
        let [got, kind, size] = parts[..] else {
            return Err(format!("git cat-file: {header}"));
        };
        if got != *oid || kind != "blob" {
            return Err(format!("git cat-file: {header}"));
        }
        let size: usize = size.parse().map_err(|_| short())?;
        let end = nl + 1 + size;
        blobs.push(out.get(nl + 1..end).ok_or_else(short)?.to_vec());
        at = end + 1;
    }
    Ok(blobs)
}

/// The committer time of `commit`, RFC 3339 UTC (read from the commit
/// object, so no log or signature config runs).
fn commit_time(git: &Git, commit: &str) -> String {
    let text = git.run(&["cat-file", "commit", commit]).unwrap_or_default();
    text.lines()
        .take_while(|l| !l.is_empty())
        .find_map(|l| l.strip_prefix("committer "))
        .and_then(|l| l.rsplit(' ').nth(1))
        .and_then(|secs| secs.parse::<i64>().ok())
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// Index commit `commit`'s blobs into the new file `out`: the same file
/// filter as a work tree's (code files, no hidden or secret-looking path,
/// no symlink or submodule, the byte cap), the same limits, and nothing
/// that varies by run (`indexedAt` is the commit's time), so the same
/// commit gives the same bytes unless a file's parse hits its time limit
/// (then named in `timedOut`).
pub(crate) fn build_base(
    git: &Git,
    commit: &str,
    out: &Path,
    limits: &Limits,
    deadline: Instant,
) -> Result<store::Built, String> {
    let (rows, skipped) = tree_rows(git, commit)?;
    let listing = Listing {
        files: rows.iter().map(|r| r.0.clone()).collect(),
        skipped,
        outside: Vec::new(),
    };
    within_limits(&listing, limits)?;
    let oids: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
    let blobs = read_blobs(git, &oids)?;
    let by_path: HashMap<&str, &Vec<u8>> = listing
        .files
        .iter()
        .zip(&blobs)
        .map(|(f, b)| (f.path.as_str(), b))
        .collect();
    let parsed = parse_with(&listing.files, deadline, &|f| {
        by_path.get(f.path.as_str()).map(|b| b.to_vec())
    })
    .map_err(|why| abort_text(why, limits))?;
    let resolved = resolve::resolve(&parsed);
    let mut h = Sha256::new();
    h.update(format!(
        "base\0{commit}\0{}\0{}",
        parse::PARSERS,
        store::SCHEMA_VERSION
    ));
    store::write(
        out,
        &parsed,
        &resolved,
        &store::Meta {
            head: commit[..9].to_string(),
            stamp: hex(&h.finalize()),
            indexed_at: commit_time(git, commit),
            skipped: listing.skipped.clone(),
            extra: vec![("kind", "base".into()), ("base", commit.to_string())],
            ..Default::default()
        },
    )
}

/// The files a base of `commit` holds, by path, each with its blob id, and
/// the paths skipped over the byte cap: code files only, no hidden or
/// secret-looking path, no symlink or submodule (the work tree's filter).
fn tree_rows(git: &Git, commit: &str) -> Result<(Vec<(Listed, String)>, Vec<String>), String> {
    let tree = git.run(&["ls-tree", "-r", "-z", "--full-tree", "-l", commit])?;
    let mut rows: Vec<(Listed, String)> = Vec::new();
    let mut skipped = Vec::new();
    for entry in tree.split('\0').filter(|e| !e.is_empty()) {
        let Some((info, path)) = entry.split_once('\t') else {
            continue;
        };
        let parts: Vec<&str> = info.split_whitespace().collect();
        let [mode, kind, oid, size] = parts[..] else {
            continue;
        };
        if kind != "blob" || !matches!(mode, "100644" | "100755") {
            continue; // a symlink, a submodule
        }
        let Some(lang) = Lang::of(path) else { continue };
        if !readable_path(path) {
            continue;
        }
        let bytes: u64 = size.parse().unwrap_or(u64::MAX);
        if bytes > MAX_FILE_BYTES {
            skipped.push(path.to_string());
            continue;
        }
        let listed = Listed {
            path: path.to_string(),
            lang,
            bytes,
            mtime_ns: 0,
        };
        rows.push((listed, oid.to_string()));
    }
    rows.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    skipped.sort();
    Ok((rows, skipped))
}

/// The overlay stamp: the work tree's (CC2's paths, sizes and times) bound
/// to the base it sits on (its commit and [`store::Index::base_identity`]:
/// its id ranges, fingerprint and timed-out files).
fn overlay_stamp(listing: &Listing, commit: &str, base: &Path) -> String {
    let mut h = Sha256::new();
    h.update(stamp(listing));
    h.update(format!("\0overlay\0{commit}\0"));
    let identity = store::Index::open(base).and_then(|b| b.base_identity());
    h.update(identity.unwrap_or_default());
    hex(&h.finalize())
}

/// The worktree's overlay on `base`, rebuilt when its files or base
/// changed. The overlay file is keyed by the worktree root.
fn ensure_overlay(
    git: &Git,
    listing: &Listing,
    dir: &Path,
    base: &Path,
    commit: &str,
    limits: &Limits,
    deadline: Instant,
) -> Result<PathBuf, String> {
    let key = short_sha(git.root.to_string_lossy().as_bytes());
    let path = dir.join(format!("{key}.overlay.sqlite"));
    let want = overlay_stamp(listing, commit, base);
    let unusable = |verdict: Option<&(String, String)>| {
        verdict
            .filter(|(stamp, _)| *stamp == want)
            .map(|(_, why)| why.clone())
    };
    let known = UNUSABLE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&path)
        .cloned();
    if let Some(why) = unusable(known.as_ref()) {
        return Err(why);
    }
    let current = || store::read_meta(&path, "stamp").as_deref() == Some(want.as_str());
    if !current() {
        let built = build_locked(&path, deadline, &current, |tmp| {
            build_overlay(git, listing, base, commit, &want, tmp, limits, deadline)
        });
        let mut cache = UNUSABLE.lock().unwrap_or_else(|e| e.into_inner());
        match built {
            Err(e) if e.starts_with(OVERLAY_UNUSABLE) => {
                cache.insert(path.clone(), (want.clone(), e.clone()));
                return Err(e);
            }
            other => {
                cache.remove(&path);
                other?;
            }
        }
    }
    Ok(path)
}

fn def_of(n: &store::NodeRow) -> parse::Def {
    parse::Def {
        kind: n.kind.clone(),
        name: n.name.clone(),
        container: n.container.clone(),
        modpath: n.modpath.clone(),
        l0: n.l0 as u32,
        l1: n.l1 as u32,
        test: n.test,
        command: n.command,
    }
}

fn edge_of(e: &store::RawEdge, first: i64) -> parse::Edge {
    parse::Edge {
        src: e
            .src
            .and_then(|s| s.checked_sub(first))
            .and_then(|d| u32::try_from(d).ok()),
        line: e.line as u32,
        kind: e.kind.clone(),
        name: e.name.clone(),
        qual: e.qual.clone(),
        macro_args: e.macro_args,
    }
}

/// Write the overlay of the work tree (`listing`) on `base` into `out`.
#[allow(clippy::too_many_arguments)]
fn build_overlay(
    git: &Git,
    listing: &Listing,
    base_path: &Path,
    commit: &str,
    stamp: &str,
    out: &Path,
    limits: &Limits,
    deadline: Instant,
) -> Result<(), String> {
    let base = store::Index::open(base_path)?;
    let bfiles = base.files_under("")?;
    let bmap: HashMap<&str, &store::FileRow> =
        bfiles.iter().map(|f| (f.path.as_str(), f)).collect();
    let listed: HashMap<&str, &Listed> =
        listing.files.iter().map(|f| (f.path.as_str(), f)).collect();

    // Which paths differ: git's diff from the base commit, plus anything
    // listed here and not in the base (untracked), or the reverse.
    let mut cand: BTreeSet<String> = changed_paths(git, commit)?.into_iter().collect();
    cand.extend(
        listing
            .files
            .iter()
            .filter(|f| !bmap.contains_key(f.path.as_str()))
            .map(|f| f.path.clone()),
    );
    cand.extend(
        bfiles
            .iter()
            .filter(|f| !listed.contains_key(f.path.as_str()))
            .map(|f| f.path.clone()),
    );
    let mut hidden: Vec<String> = Vec::new();
    let mut files: Vec<Listed> = Vec::new();
    let mut texts: HashMap<String, Vec<u8>> = HashMap::new();
    for p in &cand {
        if Lang::of(p).is_none() || !readable_path(p) {
            continue;
        }
        let was = bmap.get(p.as_str());
        let now = listed
            .get(p.as_str())
            .and_then(|l| read_inside(&git.root, p).map(|src| (*l, src)));
        match (was, now) {
            (Some(b), Some((_, src))) if short_sha(&src) == b.sha => {}
            (was, Some((l, src))) => {
                if was.is_some() {
                    hidden.push(p.clone());
                }
                files.push(l.clone());
                texts.insert(p.clone(), src);
            }
            (Some(_), None) => hidden.push(p.clone()), // a tombstone
            (None, None) => {}
        }
    }
    let tombs: Vec<&str> = hidden
        .iter()
        .filter(|h| !texts.contains_key(*h))
        .map(String::as_str)
        .collect();
    let changed = files.len() + tombs.len();
    if changed > OVERLAY_MAX_FILES && changed * 2 > bfiles.len() {
        return Err(format!(
            "{OVERLAY_UNUSABLE} {changed} of {} files differ from the base",
            bfiles.len()
        ));
    }
    let parsed = parse_with(&files, deadline, &|f| texts.get(&f.path).cloned())
        .map_err(|why| abort_text(why, limits))?;

    // The names whose resolution can change: those defined in changed
    // files (base and overlay versions), and those a base import now finds
    // in another file.
    let hidden_set: HashSet<&str> = hidden.iter().map(String::as_str).collect();
    let mut names: HashSet<String> = parsed
        .iter()
        .flat_map(|f| f.parsed.defs.iter().map(|d| d.name.clone()))
        .collect();
    for h in &hidden {
        names.extend(base.defs_in(h)?.into_iter().map(|d| d.name));
    }
    let imports = base.raw_edges("e.kind = 'import'", &[])?;
    let added: Vec<&str> = files
        .iter()
        .filter(|f| !bmap.contains_key(f.path.as_str()))
        .map(|f| f.path.as_str())
        .collect();
    if !(tombs.is_empty() && added.is_empty()) {
        let tomb_set: HashSet<&str> = tombs.iter().copied().collect();
        let base_paths: Vec<&str> = bfiles.iter().map(|f| f.path.as_str()).collect();
        let mut merged_paths: Vec<&str> = base_paths
            .iter()
            .copied()
            .filter(|p| !tomb_set.contains(p))
            .collect();
        merged_paths.extend(added.iter().copied());
        let base_by: HashMap<&str, usize> = base_paths
            .iter()
            .enumerate()
            .map(|(i, p)| (*p, i))
            .collect();
        let merged_by: HashMap<&str, usize> = merged_paths
            .iter()
            .enumerate()
            .map(|(i, p)| (*p, i))
            .collect();
        for e in &imports {
            if hidden_set.contains(e.path.as_str()) {
                continue;
            }
            let Some(lang) = bmap
                .get(e.path.as_str())
                .and_then(|f| Lang::from_str(&f.lang))
            else {
                continue;
            };
            let then =
                resolve::import_file(&e.path, lang, &e.qual, &base_by).map(|i| base_paths[i]);
            let now =
                resolve::import_file(&e.path, lang, &e.qual, &merged_by).map(|i| merged_paths[i]);
            if then != now {
                names.insert(e.name.clone());
            }
        }
    }
    let mut sorted: Vec<&String> = names.iter().collect();
    sorted.sort();
    let mut named = Vec::new();
    for n in sorted {
        named.extend(base.raw_edges("en.text = ?1", &[&n.as_str()])?);
    }

    // The resolution universe: every unchanged base file with all its
    // definitions (ids as stored) and the edges that matter, then the
    // overlay's files (ids after the base's).
    let mut defs_of: HashMap<String, Vec<store::NodeRow>> = HashMap::new();
    for d in base.all_defs()? {
        if !hidden_set.contains(d.path.as_str()) {
            defs_of.entry(d.path.clone()).or_default().push(d);
        }
    }
    let mut edges_of: HashMap<&str, BTreeMap<i64, &store::RawEdge>> = HashMap::new();
    for e in imports.iter().chain(&named) {
        if !hidden_set.contains(e.path.as_str()) {
            edges_of
                .entry(e.path.as_str())
                .or_default()
                .insert(e.rid, e);
        }
    }
    let offset = base.max_node_id()?;
    let identity = base.base_identity()?;
    let overflow = || "the base's definition ids are out of range".to_string();
    let mut universe: Vec<FileParse> = Vec::new();
    let mut firsts: Vec<i64> = Vec::new();
    let mut base_edges: Vec<Vec<&store::RawEdge>> = Vec::new();
    for f in &bfiles {
        if hidden_set.contains(f.path.as_str()) {
            continue;
        }
        let Some(lang) = Lang::from_str(&f.lang) else {
            continue;
        };
        let defs = defs_of.remove(&f.path).unwrap_or_default();
        let first = defs.first().map_or(0, |d| d.id);
        let raw: Vec<&store::RawEdge> = edges_of
            .get(f.path.as_str())
            .map(|m| m.values().copied().collect())
            .unwrap_or_default();
        universe.push(FileParse {
            path: f.path.clone(),
            lang,
            sha: f.sha.clone(),
            bytes: f.bytes as u64,
            test: f.test,
            parsed: parse::Parsed {
                defs: defs.iter().map(def_of).collect(),
                edges: raw.iter().map(|e| edge_of(e, first)).collect(),
                timed_out: false,
            },
        });
        firsts.push(first);
        base_edges.push(raw);
    }
    let n_base = universe.len();
    let mut next = offset.checked_add(1).ok_or_else(overflow)?;
    for f in parsed {
        firsts.push(next);
        next = i64::try_from(f.parsed.defs.len())
            .ok()
            .and_then(|n| next.checked_add(n))
            .ok_or_else(overflow)?;
        universe.push(f);
    }
    let res = resolve::resolve_with(&universe, &firsts);
    let mut fixes = Vec::new();
    for (fi, raw) in base_edges.iter().enumerate() {
        for (ei, e) in raw.iter().enumerate() {
            let r = &res[fi][ei];
            if names.contains(&e.name) && (r.target != e.target || r.how != e.how) {
                fixes.push((e.rid, r.target, r.how.clone()));
            }
        }
    }
    fixes.sort_by_key(|f| f.0);

    // CC2's fingerprint over the merged files.
    let own = &universe[n_base..];
    let mut fp: Vec<(&str, &str)> = universe[..n_base]
        .iter()
        .chain(own)
        .map(|f| (f.path.as_str(), f.sha.as_str()))
        .collect();
    fp.sort_unstable();
    let fingerprint = short_sha(
        fp.iter()
            .map(|(p, s)| format!("{p}\0{s}\n"))
            .collect::<String>()
            .as_bytes(),
    );
    let mut timed_out: Vec<String> = store::read_meta(base_path, "timedOut")
        .and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !hidden_set.contains(p.as_str()))
        .chain(
            own.iter()
                .filter(|f| f.parsed.timed_out)
                .map(|f| f.path.clone()),
        )
        .collect();
    timed_out.sort();
    let head = git
        .run(&["rev-parse", "--short=9", "HEAD"])
        .map(|h| h.trim().to_string())
        .unwrap_or_default();
    store::write(
        out,
        own,
        &res[n_base..],
        &store::Meta {
            head,
            stamp: stamp.to_string(),
            indexed_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            skipped: listing.skipped.clone(),
            outside: listing.outside.clone(),
            node_offset: offset,
            fingerprint: Some(fingerprint),
            extra: vec![
                ("kind", "overlay".into()),
                ("base", commit.to_string()),
                ("baseIdentity", identity),
                ("overlayFiles", changed.to_string()),
                ("timedOut", json!(timed_out).to_string()),
            ],
            overlay: Some(store::OverlayRows { hidden, fixes }),
        },
    )?;
    Ok(())
}

// ── The command line: build-base and import-base ────────────────────────

/// SHA-256 of the file at `path`, hex, and its size.
fn file_sha256(path: &Path) -> Result<(String, u64), String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    Ok((hex(&Sha256::digest(&bytes)), bytes.len() as u64))
}

/// `--name value` pairs; any other word is an error.
fn flags<'a>(args: &'a [String], known: &[&str]) -> Result<HashMap<&'a str, &'a str>, String> {
    let mut out = HashMap::new();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let name = flag.as_str();
        if !known.contains(&name) {
            return Err(format!("unknown argument `{name}`"));
        }
        let value = it.next().ok_or_else(|| format!("{name} needs a value"))?;
        out.insert(name, value.as_str());
    }
    Ok(out)
}

const BUILD_USAGE: &str =
    "usage: hollow code-context build-base --repo <path> --commit <sha> --out <file>\n";
const IMPORT_USAGE: &str = "usage: hollow code-context import-base <file>\n";

/// `hollow code-context build-base --repo <path> --commit <sha> --out
/// <file>`: index that commit's blobs into `<file>` (a pure function of the
/// commit) and print a one-line JSON receipt. Exit 0, 1 refused, 2 usage.
/// `vault`: a repository that is the home folder or holds or sits inside
/// it is refused, as the tools refuse it.
pub(crate) fn cli_build_base(args: &[String], vault: Option<&Path>) -> (String, i32) {
    let flags = match flags(args, &["--repo", "--commit", "--out"]) {
        Ok(f) if f.len() == 3 => f,
        Ok(_) => return (BUILD_USAGE.into(), 2),
        Err(e) => return (format!("build-base: {e}\n{BUILD_USAGE}"), 2),
    };
    match build_base_cli(
        Path::new(flags["--repo"]),
        flags["--commit"],
        Path::new(flags["--out"]),
        vault,
    ) {
        Ok(receipt) => (format!("{receipt}\n"), 0),
        Err(e) => (format!("build-base: {e}\n"), 1),
    }
}

fn build_base_cli(
    repo: &Path,
    commit: &str,
    out: &Path,
    vault: Option<&Path>,
) -> Result<serde_json::Value, String> {
    let started = Instant::now();
    let git = Git::open(repo)?;
    if let Some(why) = crate::code_context::root_refusal(&git.root, vault) {
        return Err(why);
    }
    let commit = full_commit(&git, commit)
        .ok_or_else(|| format!("`{commit}` is not a commit in {}", git.root.display()))?;
    let dir = match out.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let name = out
        .file_name()
        .ok_or("--out needs a file name")?
        .to_string_lossy()
        .into_owned();
    let tmp = dir.join(format!(".{name}.{}.building", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let limits = Limits {
        deadline: CLI_DEADLINE,
        ..LIMITS
    };
    let built = build_base(&git, &commit, &tmp, &limits, Instant::now() + CLI_DEADLINE);
    let built = match built {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    std::fs::rename(&tmp, out).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not write {}: {e}", out.display())
    })?;
    let (sha256, bytes) = file_sha256(out)?;
    Ok(json!({
        "commit": commit,
        "out": out.to_string_lossy(),
        "sha256": sha256,
        "bytes": bytes,
        "files": built.files,
        "nodes": built.nodes,
        "edges": built.edges,
        "skippedOverCap": built.skipped.len(),
        "timedOut": built.timed_out,
        "schema": store::SCHEMA_VERSION,
        "parsers": parse::PARSERS,
        "seconds": (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
    }))
}

/// `hollow code-context import-base <file>`: take a base built elsewhere
/// (the Mac mini) into this Mac's index folder for the checkout `root`'s
/// repository, after checking its meta: a base, this Hollow's schema and
/// parser versions, and a commit this repository has. Copied in by atomic
/// rename. Exit 0, 1 refused, 2 usage. A root that is the home folder or
/// holds or sits inside `vault` is refused, as the tools refuse it.
pub(crate) fn cli_import_base(
    args: &[String],
    root: &Path,
    cache: Option<&Path>,
    vault: Option<&Path>,
) -> (String, i32) {
    let [file] = args else {
        return (IMPORT_USAGE.into(), 2);
    };
    if file.starts_with('-') {
        return (IMPORT_USAGE.into(), 2);
    }
    match import_base(Path::new(file), root, cache, vault) {
        Ok(receipt) => (format!("{receipt}\n"), 0),
        Err(e) => (format!("import-base: {e}; nothing was imported.\n"), 1),
    }
}

pub(crate) fn import_base(
    file: &Path,
    root: &Path,
    cache: Option<&Path>,
    vault: Option<&Path>,
) -> Result<serde_json::Value, String> {
    if root.as_os_str().is_empty() {
        return Err("no checkout: run it inside the repository the base is for".into());
    }
    let git = Git::open(root)?;
    if let Some(why) = crate::code_context::root_refusal(&git.root, vault) {
        return Err(why);
    }
    let dir = cache_dir(cache).ok_or("no cache folder for the code index")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create the index folder: {e}"))?;
    let key = repo_key(&git)?;
    // Check a private copy, so what is checked is what is swapped in.
    let tmp = dir.join(format!(
        ".import.{}.{}.building",
        std::process::id(),
        BUILD_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let checked = copy_in(file, &tmp)
        .and_then(|_| check_import(&tmp, &git))
        .and_then(|commit| {
            let dest = base_path(&dir, &key, &commit);
            let replaced = dest.exists();
            std::fs::rename(&tmp, &dest)
                .map(|_| (commit, dest, replaced))
                .map_err(|e| format!("could not swap it in: {e}"))
        });
    let (commit, dest, replaced) = match checked {
        Ok(done) => done,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    prune_bases(&dir, &key, &dest);
    let (sha256, bytes) = file_sha256(&dest)?;
    Ok(json!({
        "imported": dest.to_string_lossy(),
        "commit": commit,
        "sha256": sha256,
        "bytes": bytes,
        "replaced": replaced,
    }))
}

/// Copy the base at `file` to the new private file `tmp` (mode 0644, so
/// SQLite's check and a later touch can write it): only a regular file,
/// its final part not a symlink, opened without blocking (a FIFO), at most
/// [`MAX_IMPORT_BYTES`].
fn copy_in(file: &Path, tmp: &Path) -> Result<(), String> {
    use std::io::Read;
    let cant = |e: std::io::Error| format!("could not read {}: {e}", file.display());
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let src = open.open(file).map_err(cant)?;
    let meta = src.metadata().map_err(cant)?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", file.display()));
    }
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(format!(
            "it is {} bytes, over the {MAX_IMPORT_BYTES}-byte cap for a base",
            meta.len()
        ));
    }
    let mut create = std::fs::OpenOptions::new();
    create.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        create.mode(0o644);
    }
    let mut out = create
        .open(tmp)
        .map_err(|e| format!("could not copy it in: {e}"))?;
    let copied = std::io::copy(&mut src.take(MAX_IMPORT_BYTES + 1), &mut out)
        .map_err(|e| format!("could not copy it in: {e}"))?;
    if copied > MAX_IMPORT_BYTES {
        return Err(format!(
            "it is over the {MAX_IMPORT_BYTES}-byte cap for a base"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(0o644))
            .map_err(|e| format!("could not copy it in: {e}"))?;
    }
    Ok(())
}

/// The checks of an imported base (`file`, a private copy): exactly a
/// base's tables and consistent ids ([`store::check_base_tables`], with
/// views and triggers refused before anything reads it), SQLite's check, a
/// base, this Hollow's schema and parsers, a commit `git`'s repository has,
/// and exactly that commit's code files (each path readable, of its
/// language and size). Returns the commit.
fn check_import(file: &Path, git: &Git) -> Result<String, String> {
    store::check_base_tables(file)?;
    store::quick_check(file)?;
    let index = store::Index::open(file)?;
    let get = |k: &str| index.meta(k).unwrap_or_default();
    if get("kind") != "base" {
        return Err("it is not a base index".into());
    }
    let schema = get("schema");
    if schema != store::SCHEMA_VERSION.to_string() {
        return Err(format!(
            "it was built with index schema `{schema}` and this Hollow reads schema {}",
            store::SCHEMA_VERSION
        ));
    }
    let parsers = get("parsers");
    if parsers != parse::PARSERS {
        return Err(format!(
            "it was built with parsers `{parsers}` and this Hollow has `{}`",
            parse::PARSERS
        ));
    }
    let commit = get("base");
    if !is_sha(&commit) || full_commit(git, &commit).as_deref() != Some(commit.as_str()) {
        return Err(format!(
            "its commit `{commit}` is not in the repository at {}",
            git.root.display()
        ));
    }
    let files = index.files_under("")?;
    for f in &files {
        let lang_ok = Lang::of(&f.path).is_some_and(|l| Lang::from_str(&f.lang) == Some(l));
        if !readable_path(&f.path) || !lang_ok {
            return Err(format!(
                "it names a file this Hollow would never index (`{}`)",
                f.path.escape_debug()
            ));
        }
    }
    let (rows, _) = tree_rows(git, &commit)?;
    let want: Vec<(&str, u64)> = rows
        .iter()
        .map(|(l, _)| (l.path.as_str(), l.bytes))
        .collect();
    let got: Vec<(&str, u64)> = files
        .iter()
        .map(|f| (f.path.as_str(), u64::try_from(f.bytes).unwrap_or(u64::MAX)))
        .collect();
    if got != want {
        return Err(format!(
            "its files are not commit {}'s code files ({} listed, the commit has {})",
            &commit[..9],
            got.len(),
            want.len()
        ));
    }
    Ok(commit)
}

#[cfg(test)]
#[path = "overlay_tests.rs"]
mod tests;
