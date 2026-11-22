//! iCloud/Finder conflict copies in the vault (overnight A10). When two
//! Macs, or iCloud and an open editor, write a note at once, macOS keeps
//! both: `Note 2.md` beside `Note.md` (Finder and iCloud), `Note (1).md`
//! (downloads, Google Drive) or `Note (conflicted copy 2026-10-09).md`
//! (Dropbox and others). [`scan`] lists every such file whose original sits
//! beside it; the nightly gardener names them and Waiting on you offers
//! them. Resolving is the person's click: keep the original (the copy goes
//! to the vault's `.trash/`), or keep the copy (the original goes to
//! `.trash/` and the copy takes its name). Nothing is ever deleted, and a
//! file that changed since the person saw the diff refuses. A pair the
//! person marks **Not a copy** (`Chapter 2.md` beside `Chapter.md`) is never
//! offered again ([`dismiss`], morning M7): the marks live outside the vault,
//! in `<support>/ConflictCopies/<vault>.json`, by both paths, and
//! [`undismiss`] takes one back.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

/// One copy and its original.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ConflictCopy {
    /// Vault-relative paths.
    pub copy: String,
    pub original: String,
    /// The two files hold the same bytes.
    pub identical: bool,
    pub copy_modified: String,
    pub original_modified: String,
    pub copy_size: u64,
    pub original_size: u64,
    /// The original is a protected file (`run_guard`: AGENTS.md, me/…):
    /// keeping the copy over it is refused; compare and edit it by hand.
    pub original_protected: bool,
}

/// Both texts and their hashes, for the person's diff; the resolve binds
/// the hashes.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ConflictPair {
    pub copy: String,
    pub original: String,
    pub copy_text: String,
    pub original_text: String,
    pub copy_hash: String,
    pub original_hash: String,
}

/// The original's file name if `name` is a conflict copy's, else `None`.
/// `Note 2.md` (2–20), `Note (1).md` (1–20), `Note (conflicted copy …).md`,
/// `Note (Mac's conflicted copy …).md` and the context sync's
/// `Note (from <machine slug>).md` (CX6a).
pub fn original_name(name: &str) -> Option<String> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    // No leading zero: `Note 02.md` is a numbered title, not a copy.
    let small = |s: &str, lo: u32| {
        s.len() <= 2
            && !s.starts_with('0')
            && s.parse::<u32>().is_ok_and(|n| (lo..=20).contains(&n))
    };
    // `Note (…)`
    if let Some(open) = stem
        .strip_suffix(')')
        .and_then(|s| s.rfind(" (").map(|i| (i, s)))
    {
        let (i, s) = open;
        let inner = &s[i + 2..];
        let base = &s[..i];
        // `Note (from <machine>).md` and `Note (from <machine> 2).md`: the
        // other Mac's version the context sync kept beside a clash (CX6a).
        let synced = inner.strip_prefix("from ").is_some_and(|rest| {
            let (slug, n) = rest.split_once(' ').unwrap_or((rest, "2"));
            crate::context_roots::valid_slug(slug).is_ok() && small(n, 2)
        });
        if !base.trim().is_empty()
            && (small(inner, 1) || inner.contains("conflicted copy") || synced)
        {
            return Some(format!("{base}{ext}"));
        }
    }
    // `Note 2`
    if let Some(i) = stem.rfind(' ') {
        let (base, n) = (&stem[..i], &stem[i + 1..]);
        if !base.trim().is_empty() && small(n, 2) {
            return Some(format!("{base}{ext}"));
        }
    }
    None
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Every conflict copy in the vault whose original sits beside it, by path,
/// with `identical` read.
pub fn scan(vault: &Path) -> Vec<ConflictCopy> {
    scan_with(vault, true)
}

/// [`scan`]; `compare` false skips reading the files (the bell's query runs
/// often, and reading an evicted iCloud file downloads it): `identical` is
/// then false.
pub fn scan_with(vault: &Path, compare: bool) -> Vec<ConflictCopy> {
    scan_notes(vault, crate::vault::walk_notes(vault), compare)
}

fn scan_notes(vault: &Path, notes: Vec<crate::types::Note>, compare: bool) -> Vec<ConflictCopy> {
    let by_path: std::collections::HashMap<&str, &crate::types::Note> =
        notes.iter().map(|n| (n.path.as_str(), n)).collect();
    let not_copies: std::collections::HashSet<(String, String)> = load_marks(vault)
        .into_iter()
        .map(|m| (m.copy, m.original))
        .collect();
    let mut out = Vec::new();
    for note in &notes {
        let file = note.path.rsplit('/').next().unwrap_or(&note.path);
        let Some(orig_name) = original_name(file) else {
            continue;
        };
        let original = match note.path.rfind('/') {
            Some(i) => format!("{}/{orig_name}", &note.path[..i]),
            None => orig_name,
        };
        let Some(orig) = by_path.get(original.as_str()) else {
            continue;
        };
        if not_copies.contains(&(note.path.clone(), original.clone())) {
            continue;
        }
        let identical = compare
            && note.size == orig.size
            && std::fs::read(vault.join(&note.path)).ok()
                == std::fs::read(vault.join(&original)).ok();
        let original_protected = crate::run_guard::is_protected_path(&original);
        out.push(ConflictCopy {
            copy: note.path.clone(),
            original,
            identical,
            copy_modified: note.mtime.clone(),
            original_modified: orig.mtime.clone(),
            copy_size: note.size,
            original_size: orig.size,
            original_protected,
        });
    }
    out.sort_by(|a, b| a.copy.cmp(&b.copy));
    out
}

/// Both texts and hashes of `copy` and its original.
pub fn pair(vault: &Path, copy: &str) -> Result<ConflictPair, String> {
    let found = scan(vault)
        .into_iter()
        .find(|c| c.copy == copy)
        .ok_or_else(|| format!("{copy} is not a conflict copy with its original beside it."))?;
    let read = |rel: &str| std::fs::read(vault.join(rel)).map_err(|e| format!("read {rel}: {e}"));
    let (c, o) = (read(&found.copy)?, read(&found.original)?);
    Ok(ConflictPair {
        copy_hash: hash(&c),
        original_hash: hash(&o),
        copy_text: String::from_utf8_lossy(&c).into_owned(),
        original_text: String::from_utf8_lossy(&o).into_owned(),
        copy: found.copy,
        original: found.original,
    })
}

/// The person keeps one: `keep_copy` false trashes the copy; true keeps the
/// original in `.trash/` and renames the copy into its place in one atomic
/// step (the name is never missing). Both files must still hold what the
/// person saw (`copy_hash`, `original_hash`), checked under the edit lock.
/// Refused while a run holds the vault or a conversation runs in the
/// workspace holding the files, and keeping a copy over a protected original.
/// Returns the path kept.
pub fn resolve(
    vault: &Path,
    copy: &str,
    keep_copy: bool,
    copy_hash: &str,
    original_hash: &str,
) -> Result<String, String> {
    resolve_as(vault, copy, keep_copy, (copy_hash, original_hash), true)
}

/// [`resolve`] on behalf of the run that holds the vault (the COO's
/// `vault.resolve_conflict_copy`, L13): the same checks and move, called
/// inside that run's `with_live_run`, so the move is recorded as the run's
/// change (History can put it back) instead of refusing on the edit lease,
/// as `workspaces::trash_for_live_run` does.
pub(crate) fn resolve_for_live_run(
    vault: &Path,
    copy: &str,
    keep_copy: bool,
    copy_hash: &str,
    original_hash: &str,
) -> Result<String, String> {
    resolve_as(vault, copy, keep_copy, (copy_hash, original_hash), false)
}

fn resolve_as(
    vault: &Path,
    copy: &str,
    keep_copy: bool,
    (copy_hash, original_hash): (&str, &str),
    edit_lease: bool,
) -> Result<String, String> {
    let found = scan_with(vault, false)
        .into_iter()
        .find(|c| c.copy == copy)
        .ok_or_else(|| format!("{copy} is not a conflict copy with its original beside it."))?;
    if keep_copy && found.original_protected {
        return Err(format!(
            "{} is a protected file: compare the two and edit it by hand. Nothing was moved.",
            found.original
        ));
    }
    // A conversation running in the workspace holds its folder, not the vault
    // lease (as trashing a workspace checks).
    let mut parts = found.copy.split('/');
    if let (Some("workspaces"), Some(id)) = (parts.next(), parts.next()) {
        if crate::automated_history::scope_holder(vault, &format!("workspaces/{id}")).is_some() {
            return Err("A conversation in this workspace is still running. Resolve it when the turn finishes; nothing was moved.".into());
        }
    }
    let _lease = if edit_lease {
        Some(crate::automated_history::workspace_edit_lease(vault).map_err(String::from)?)
    } else {
        None
    };
    let check = |c: &[u8], o: &[u8]| {
        if hash(c) != copy_hash || hash(o) != original_hash {
            Err(
                "One of the two files changed since you saw them. Nothing was moved; look again."
                    .to_string(),
            )
        } else {
            Ok(())
        }
    };
    if keep_copy {
        crate::vault::replace_from(vault, &found.copy, &found.original, check)?;
    } else {
        let _guard = crate::vault::lock_mutate();
        let read =
            |rel: &str| std::fs::read(vault.join(rel)).map_err(|e| format!("read {rel}: {e}"));
        check(&read(&found.copy)?, &read(&found.original)?)?;
        crate::vault::delete_note(vault, &found.copy)?;
    }
    Ok(found.original)
}

/// A pair the person marked "Not a copy" (morning M7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NotACopy {
    copy: String,
    original: String,
    at: String,
}

/// Marks kept, newest last.
const MARKS_KEPT: usize = 500;

fn marks_path(vault: &Path) -> Option<std::path::PathBuf> {
    let stem = crate::session_store::vault_file_stem(vault);
    #[cfg(test)]
    {
        Some(crate::test_temp::process_dir("hollow-test-not-copies-").join(format!("{stem}.json")))
    }
    #[cfg(not(test))]
    {
        crate::instance::support_dir()
            .map(|r| r.join("ConflictCopies").join(format!("{stem}.json")))
    }
}

static MARKS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// Bumped by every change to the marks ([`scan_cached`]'s key).
static MARKS_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// True when the person marked `copy` (beside `original`) Not a copy.
pub(crate) fn is_not_a_copy(vault: &Path, copy: &str, original: &str) -> bool {
    let (copy, original) = (copy.to_lowercase(), original.to_lowercase());
    load_marks(vault)
        .iter()
        .any(|m| m.copy.to_lowercase() == copy && m.original.to_lowercase() == original)
}

fn load_marks(vault: &Path) -> Vec<NotACopy> {
    let Some(p) = marks_path(vault) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return Vec::new();
    };
    match serde_json::from_str(&text) {
        Ok(marks) => marks,
        Err(e) => {
            // Set aside, not overwritten by the next mark.
            eprintln!("[conflict_copies] {}: {e}; set aside as .bad", p.display());
            let _ = std::fs::rename(&p, p.with_extension("json.bad"));
            Vec::new()
        }
    }
}

/// Drop marks whose copy or original is gone: a later real copy with the
/// same name (iCloud names them alike) is offered again.
fn prune_gone(vault: &Path, marks: &mut Vec<NotACopy>) {
    marks.retain(|m| vault.join(&m.copy).exists() && vault.join(&m.original).exists());
}

fn save_marks(vault: &Path, marks: &[NotACopy]) -> Result<(), String> {
    let p = marks_path(vault).ok_or("No app data folder.")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let keep = &marks[marks.len().saturating_sub(MARKS_KEPT)..];
    let text = serde_json::to_vec_pretty(keep).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(&p, &text)?;
    // The bell's cached scan holds the old list.
    MARKS_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut g) = CACHE.lock() {
        *g = None;
    }
    Ok(())
}

/// The person says `copy` is not a conflict copy of the original beside it
/// (a real title such as `Chapter 2.md`): it is never offered again, while
/// both keep those paths. Nothing in the vault changes. Returns the original.
pub fn dismiss(vault: &Path, copy: &str) -> Result<String, String> {
    let found = scan_with(vault, false)
        .into_iter()
        .find(|c| c.copy == copy)
        .ok_or_else(|| format!("{copy} is not a conflict copy with its original beside it."))?;
    let _one = MARKS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut marks = load_marks(vault);
    prune_gone(vault, &mut marks);
    marks.retain(|m| m.copy != found.copy);
    marks.push(NotACopy {
        copy: found.copy,
        original: found.original.clone(),
        at: chrono::Utc::now().to_rfc3339(),
    });
    save_marks(vault, &marks)?;
    Ok(found.original)
}

/// Take back a "Not a copy" mark on `copy` (the dialog's Undo).
pub fn undismiss(vault: &Path, copy: &str) -> Result<(), String> {
    let _one = MARKS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut marks = load_marks(vault);
    let before = marks.len();
    marks.retain(|m| m.copy != copy);
    if marks.len() == before {
        return Err(format!("{copy} isn't marked as not a copy."));
    }
    prune_gone(vault, &mut marks);
    save_marks(vault, &marks)
}

/// The bell's scan, reused while the vault is unchanged (overnight A14: a
/// full walk of a 30,000-note vault on every Waiting on you query cost about
/// 130 ms). Valid for the same vault and [`crate::vault::walk_generation`],
/// at most [`CACHE_TTL`] old (a safety net for a change nothing reported).
static CACHE: std::sync::Mutex<
    Option<(
        std::path::PathBuf,
        (u64, u64),
        std::time::Instant,
        Vec<ConflictCopy>,
    )>,
> = std::sync::Mutex::new(None);
/// Short enough for a change the watcher doesn't report (it skips `_`
/// folders the walk includes); one walk a minute at most is cheap.
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// [`scan_with`] (files unread), from [`CACHE`] when it is still valid; the
/// flag says whether it was.
pub(crate) fn scan_cached(vault: &Path) -> (Vec<ConflictCopy>, bool) {
    // The marks are outside the vault, so the walk generation never sees
    // them: their own generation is part of the key, read before the walk,
    // so a scan that loaded the marks before a dismiss can't be served after
    // it (morning M7 review).
    let generation = (
        crate::vault::walk_generation(),
        MARKS_GEN.load(std::sync::atomic::Ordering::SeqCst),
    );
    if let Ok(g) = CACHE.lock() {
        if let Some((root, gen, at, found)) = g.as_ref() {
            if root == vault && *gen == generation && at.elapsed() < CACHE_TTL {
                return (found.clone(), true);
            }
        }
    }
    // A walk of its own (not the 2 s walk cache, which may hold a list from
    // before the change that bumped the generation).
    let found = scan_notes(vault, crate::vault::walk_notes_uncached(vault), false);
    if let Ok(mut g) = CACHE.lock() {
        *g = Some((
            vault.to_path_buf(),
            generation,
            std::time::Instant::now(),
            found.clone(),
        ));
    }
    (found, false)
}

/// The Waiting on you item's (title, detail), when there is anything,
/// leaving out the copies in `skip` (a context sync clash's copy has its own
/// card, CX6c).
pub fn waiting(
    vault: &Path,
    skip: &std::collections::BTreeSet<String>,
) -> Option<(String, String)> {
    let (mut found, _) = scan_cached(vault);
    found.retain(|c| !skip.contains(&c.copy));
    if found.is_empty() {
        return None;
    }
    let n = found.len();
    let title = format!(
        "{n} conflict cop{} in the vault",
        if n == 1 { "y" } else { "ies" }
    );
    let mut detail: Vec<String> = found.iter().take(5).map(|c| c.copy.clone()).collect();
    if n > 5 {
        detail.push(format!("and {} more", n - 5));
    }
    Some((title, detail.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn copy_names_map_to_their_originals() {
        assert_eq!(
            original_name("Note 02.md"),
            None,
            "a leading zero is a numbered title"
        );
        for (copy, orig) in [
            ("Note 2.md", "Note.md"),
            ("Weekly plan 3.md", "Weekly plan.md"),
            ("Note (1).md", "Note.md"),
            ("Note (conflicted copy 2026-10-09).md", "Note.md"),
            (
                "Note (Alex's MacBook Pro's conflicted copy 2026-10-09).md",
                "Note.md",
            ),
            ("page 2.html", "page.html"),
            ("Identity (from mac-mini).md", "Identity.md"),
            ("Identity (from mac-mini 3).md", "Identity.md"),
        ] {
            assert_eq!(original_name(copy).as_deref(), Some(orig), "{copy}");
        }
        for name in [
            "Note.md",
            "Chapter 21.md",
            "2.md",
            " 2.md",
            "Note (draft).md",
            "Note 1.md",
            "Q3 2026.md",
            "(1).md",
            "Note (from Somewhere).md",
            "Note (from mac-mini 30).md",
        ] {
            assert_eq!(original_name(name), None, "{name}");
        }
    }

    fn vault() -> tempfile::TempDir {
        // Not `.tmp…`: the vault walk skips hidden folders, its root too.
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        fs::create_dir_all(dir.path().join("notes")).unwrap();
        dir
    }

    #[test]
    fn scan_lists_copies_whose_original_sits_beside_them() {
        let dir = vault();
        let v = dir.path();
        fs::write(v.join("notes/Plan.md"), "a\n").unwrap();
        fs::write(v.join("notes/Plan 2.md"), "a\nb\n").unwrap();
        fs::write(v.join("notes/Same.md"), "x").unwrap();
        fs::write(v.join("notes/Same (1).md"), "x").unwrap();
        fs::write(v.join("notes/Lonely 2.md"), "no original").unwrap();
        fs::write(v.join("Chapter 2.md"), "a legit title, no Chapter.md").unwrap();
        crate::vault::invalidate_walk_cache();
        let found = scan(v);
        assert_eq!(
            found
                .iter()
                .map(|c| (c.copy.as_str(), c.original.as_str(), c.identical))
                .collect::<Vec<_>>(),
            [
                ("notes/Plan 2.md", "notes/Plan.md", false),
                ("notes/Same (1).md", "notes/Same.md", true)
            ]
        );
        let (title, detail) = waiting(v, &Default::default()).unwrap();
        assert_eq!(title, "2 conflict copies in the vault");
        assert_eq!(detail, "notes/Plan 2.md; notes/Same (1).md");
    }

    #[test]
    fn keeping_the_original_trashes_the_copy_and_keeping_the_copy_takes_its_name() {
        let dir = vault();
        let v = dir.path();
        fs::write(v.join("notes/Plan.md"), "old\n").unwrap();
        fs::write(v.join("notes/Plan 2.md"), "new\n").unwrap();
        fs::write(v.join("notes/Idea.md"), "keep me\n").unwrap();
        fs::write(v.join("notes/Idea 2.md"), "stray\n").unwrap();
        crate::vault::invalidate_walk_cache();

        let p = pair(v, "notes/Plan 2.md").unwrap();
        assert_eq!(
            (p.original_text.as_str(), p.copy_text.as_str()),
            ("old\n", "new\n")
        );
        // Changed since the person saw it: refused, nothing moved.
        fs::write(v.join("notes/Plan 2.md"), "newer\n").unwrap();
        let err = resolve(v, "notes/Plan 2.md", true, &p.copy_hash, &p.original_hash).unwrap_err();
        assert!(err.contains("changed since you saw them"), "{err}");
        assert_eq!(
            fs::read_to_string(v.join("notes/Plan.md")).unwrap(),
            "old\n"
        );

        crate::vault::invalidate_walk_cache();
        let p = pair(v, "notes/Plan 2.md").unwrap();
        assert_eq!(
            resolve(v, "notes/Plan 2.md", true, &p.copy_hash, &p.original_hash).unwrap(),
            "notes/Plan.md"
        );
        assert_eq!(
            fs::read_to_string(v.join("notes/Plan.md")).unwrap(),
            "newer\n"
        );
        assert!(!v.join("notes/Plan 2.md").exists());

        crate::vault::invalidate_walk_cache();
        let p = pair(v, "notes/Idea 2.md").unwrap();
        resolve(v, "notes/Idea 2.md", false, &p.copy_hash, &p.original_hash).unwrap();
        assert_eq!(
            fs::read_to_string(v.join("notes/Idea.md")).unwrap(),
            "keep me\n"
        );
        assert!(!v.join("notes/Idea 2.md").exists());

        // Nothing was deleted: both losers are in the vault's trash.
        let trashed: Vec<String> = fs::read_dir(v.join(".trash"))
            .unwrap()
            .flatten()
            .map(|e| fs::read_to_string(e.path()).unwrap())
            .collect();
        assert_eq!(trashed.len(), 2, "{trashed:?}");
        assert!(trashed.contains(&"old\n".to_string()) && trashed.contains(&"stray\n".to_string()));
        crate::vault::invalidate_walk_cache();
        assert!(scan(v).is_empty() && waiting(v, &Default::default()).is_none());
    }

    #[test]
    fn a_file_that_is_not_a_copy_is_refused() {
        let dir = vault();
        let v = dir.path();
        fs::write(v.join("notes/Plan.md"), "a").unwrap();
        crate::vault::invalidate_walk_cache();
        assert!(pair(v, "notes/Plan.md")
            .unwrap_err()
            .contains("not a conflict copy"));
        assert!(resolve(v, "../outside 2.md", false, "", "").is_err());
    }
    #[test]
    fn keeping_a_copy_over_a_protected_original_is_refused() {
        let dir = vault();
        let v = dir.path();
        fs::create_dir_all(v.join("workspaces/w1")).unwrap();
        fs::write(v.join("workspaces/w1/AGENTS.md"), "rules\n").unwrap();
        fs::write(v.join("workspaces/w1/AGENTS 2.md"), "other rules\n").unwrap();
        crate::vault::invalidate_walk_cache();
        let c = scan(v)
            .into_iter()
            .find(|c| c.copy == "workspaces/w1/AGENTS 2.md")
            .unwrap();
        assert!(c.original_protected);
        let p = pair(v, &c.copy).unwrap();
        let err = resolve(v, &c.copy, true, &p.copy_hash, &p.original_hash).unwrap_err();
        assert!(err.contains("protected file"), "{err}");
        assert_eq!(
            fs::read_to_string(v.join("workspaces/w1/AGENTS.md")).unwrap(),
            "rules\n"
        );
        // Keeping the original (trashing the stray copy) is fine.
        resolve(v, &c.copy, false, &p.copy_hash, &p.original_hash).unwrap();
        assert!(!v.join("workspaces/w1/AGENTS 2.md").exists());
    }

    #[test]
    fn keeping_the_copy_replaces_the_original_in_one_step_and_keeps_it_in_trash() {
        let dir = vault();
        let v = dir.path();
        fs::write(v.join("notes/Plan.md"), "old\n").unwrap();
        fs::write(v.join("notes/Plan 2.md"), "new\n").unwrap();
        crate::vault::invalidate_walk_cache();
        let p = pair(v, "notes/Plan 2.md").unwrap();
        // replace_from's check sees both files' bytes under the lock.
        let seen = std::cell::RefCell::new(None);
        crate::vault::replace_from(v, "notes/Plan 2.md", "notes/Plan.md", |c, o| {
            *seen.borrow_mut() = Some((c.to_vec(), o.to_vec()));
            Err("refused by the check".into())
        })
        .unwrap_err();
        assert_eq!(
            seen.into_inner().unwrap(),
            (b"new\n".to_vec(), b"old\n".to_vec())
        );
        assert!(
            v.join("notes/Plan 2.md").exists() && !v.join(".trash").exists(),
            "a refusal moves nothing"
        );
        resolve(v, "notes/Plan 2.md", true, &p.copy_hash, &p.original_hash).unwrap();
        assert_eq!(
            fs::read_to_string(v.join("notes/Plan.md")).unwrap(),
            "new\n"
        );
        let trashed: Vec<_> = fs::read_dir(v.join(".trash")).unwrap().flatten().collect();
        assert_eq!(trashed.len(), 1);
        assert_eq!(fs::read_to_string(trashed[0].path()).unwrap(), "old\n");
    }

    /// Overnight A14: the bell reuses its scan until the vault changes.
    #[test]
    fn the_bell_reuses_its_scan_until_the_vault_changes() {
        let dir = vault();
        let v = dir.path();
        fs::write(v.join("notes/Plan.md"), "a").unwrap();
        fs::write(v.join("notes/Plan 2.md"), "b").unwrap();
        crate::vault::invalidate_walk_cache();
        // Another test's write may bump the generation between two calls;
        // an immediate second call is a hit in practice.
        let reused = (0..20).any(|_| {
            crate::vault::invalidate_walk_cache();
            let (first, hit1) = scan_cached(v);
            let (second, hit2) = scan_cached(v);
            !hit1 && hit2 && first == second && first.len() == 1
        });
        assert!(reused, "a second scan right after the first reuses it");
        fs::write(v.join("notes/Idea.md"), "c").unwrap();
        fs::write(v.join("notes/Idea 2.md"), "d").unwrap();
        crate::vault::invalidate_walk_cache();
        assert_eq!(
            waiting(v, &Default::default()).unwrap().0,
            "2 conflict copies in the vault",
            "a change is seen"
        );
    }

    /// Morning M7: "Not a copy" stops a pair from being offered (the scan,
    /// the bell's cached scan, Waiting on you) and nothing in the vault
    /// changes; Undo takes it back; only a real pair can be marked.
    #[test]
    fn a_pair_marked_not_a_copy_is_never_offered_again() {
        let dir = vault();
        let v = dir.path();
        fs::write(v.join("notes/Chapter.md"), "part one").unwrap();
        fs::write(v.join("notes/Chapter 2.md"), "part two").unwrap();
        fs::write(v.join("notes/Plan.md"), "a").unwrap();
        fs::write(v.join("notes/Plan 2.md"), "b").unwrap();
        crate::vault::invalidate_walk_cache();
        assert_eq!(scan(v).len(), 2);
        assert_eq!(scan_cached(v).0.len(), 2);
        assert_eq!(
            dismiss(v, "notes/Chapter 2.md").unwrap(),
            "notes/Chapter.md"
        );
        let left: Vec<String> = scan(v).into_iter().map(|c| c.copy).collect();
        assert_eq!(left, ["notes/Plan 2.md"]);
        assert_eq!(
            scan_cached(v).0.len(),
            1,
            "the cached scan forgets it at once"
        );
        assert!(waiting(v, &Default::default())
            .unwrap()
            .0
            .starts_with("1 conflict copy"));
        assert!(
            v.join("notes/Chapter 2.md").exists(),
            "nothing in the vault moved"
        );
        assert!(dismiss(v, "notes/Chapter.md").is_err(), "not a copy at all");
        assert!(dismiss(v, "notes/Chapter 2.md").is_err(), "already marked");
        undismiss(v, "notes/Chapter 2.md").unwrap();
        assert_eq!(scan(v).len(), 2, "Undo offers it again");
        assert!(undismiss(v, "notes/Chapter 2.md").is_err());
        // A mark is by both paths: the same name in another folder is offered.
        dismiss(v, "notes/Plan 2.md").unwrap();
        fs::create_dir_all(v.join("other")).unwrap();
        fs::write(v.join("other/Plan.md"), "a").unwrap();
        fs::write(v.join("other/Plan 2.md"), "b").unwrap();
        crate::vault::invalidate_walk_cache();
        let left: Vec<String> = scan(v).into_iter().map(|c| c.copy).collect();
        assert_eq!(left, ["notes/Chapter 2.md", "other/Plan 2.md"]);
        // A mark whose file went is dropped at the next change: a later real
        // copy with that name is offered again.
        fs::remove_file(v.join("notes/Plan 2.md")).unwrap();
        crate::vault::invalidate_walk_cache();
        dismiss(v, "other/Plan 2.md").unwrap();
        fs::write(v.join("notes/Plan 2.md"), "a real copy").unwrap();
        crate::vault::invalidate_walk_cache();
        let left: Vec<String> = scan(v).into_iter().map(|c| c.copy).collect();
        assert_eq!(left, ["notes/Chapter 2.md", "notes/Plan 2.md"]);
        // A damaged marks file is set aside, never silently overwritten.
        let p = marks_path(v).unwrap();
        fs::write(&p, "{ not json").unwrap();
        assert!(load_marks(v).is_empty());
        assert!(p.with_extension("json.bad").exists());
    }
}
