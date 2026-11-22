//! Clash cards of the context sync engine (CX6c): one record per clash, the
//! same for every root.
//!
//! **The record.** When a merge keeps this Mac's version of a file at its
//! name and the other Mac's beside it (`Name (from <machine>).md`), the
//! cycle saves a [`ClashRecord`] in that root's `<id>.state.json` (app data,
//! never the vault): the file, the copy, both machines, when, and a SHA-256
//! of each version as the merge wrote it. It survives restarts, and it is
//! dropped once its copy is gone (the person or the other Mac removed it).
//!
//! **The card.** Waiting on you lists one `clash` item per open record
//! (category `context-sync`; it replaced the interim error notification),
//! and `sync_clashes` gives the card list: both versions' first lines and a
//! line count of what differs, read through the no-symlink walk. The
//! person's four choices ([`resolve`]):
//! - **Keep mine**: the copy moves to the vault's `.trash/`; the file stays.
//! - **Keep theirs**: only while the file still holds this Mac's version as
//!   recorded (else it is refused, naming the change): the copy takes the
//!   file's name and the file's old text goes to `.trash/`.
//! - **Keep both**: both files stay; the record is closed.
//! - **Edit**: the record is marked editing and the file opens; **Done**
//!   (or the copy disappearing) closes it.
//!
//! A resolution is an ordinary local change in the work tree: the next cycle
//! stages, scans and commits it like any edit. An open clash never stops a
//! cycle, and resolving one touches only its two files and the record.
//! Agents read the list without contents (`settings.sync_clashes`); the
//! choices are the person's (`resolve_sync_clash`, user-only).

use super::{load_state, roots, save_state, Root, SavedState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;

/// One clash a merge left (see the module notes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClashRecord {
    /// Work-tree relative: the file holding this Mac's version.
    pub path: String,
    /// Work-tree relative: the other Mac's version beside it.
    pub copy: String,
    pub mine_machine: String,
    pub theirs_machine: String,
    /// When the merge made it (RFC 3339).
    pub at: String,
    /// SHA-256 of this Mac's version as the merge left it at `path`.
    pub mine_hash: String,
    /// SHA-256 of the other Mac's version as written to `copy`.
    pub theirs_hash: String,
    /// False for a file set aside because a folder took its name: only Keep
    /// both and Edit apply.
    #[serde(default)]
    pub replaceable: bool,
    /// The person chose Edit and hasn't said Done.
    #[serde(default)]
    pub editing: bool,
}

/// One version's first lines on the card (text only, capped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ClashSide {
    /// Up to [`PREVIEW_LINES`] lines, each at most [`PREVIEW_CHARS`] chars.
    pub lines: Vec<String>,
    /// Lines in the whole file.
    pub total_lines: u32,
    /// Why there is no preview (missing, too large, not text), when so.
    pub unreadable: Option<String>,
}

/// An open clash as the card shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SyncClash {
    /// The root's id: `personal` or `vault`.
    pub root: String,
    pub path: String,
    pub copy: String,
    pub mine_machine: String,
    pub theirs_machine: String,
    pub at: String,
    pub replaceable: bool,
    pub editing: bool,
    /// The file no longer holds this Mac's version as recorded: Keep theirs
    /// would refuse (read for the person's card only; false in the agents'
    /// list).
    pub changed: bool,
    /// This Mac's version (`path`); `None` in the agents' list.
    pub mine: Option<ClashSide>,
    /// The other Mac's version (`copy`); `None` in the agents' list.
    pub theirs: Option<ClashSide>,
    /// Lines only in this Mac's version, and only in the other's.
    pub only_mine: u32,
    pub only_theirs: u32,
}

/// The person's choice on a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClashChoice {
    KeepMine,
    KeepTheirs,
    KeepBoth,
    Edit,
    Done,
}

impl ClashChoice {
    pub(crate) fn parse(s: &str) -> Result<ClashChoice, String> {
        Ok(match s {
            "keep_mine" => ClashChoice::KeepMine,
            "keep_theirs" => ClashChoice::KeepTheirs,
            "keep_both" => ClashChoice::KeepBoth,
            "edit" => ClashChoice::Edit,
            "done" => ClashChoice::Done,
            other => {
                return Err(format!(
                    "{other:?} is not a choice: keep_mine, keep_theirs, keep_both, edit or done."
                ))
            }
        })
    }
}

/// Lines a preview shows.
pub(crate) const PREVIEW_LINES: usize = 8;
/// Characters kept of one preview line.
pub(crate) const PREVIEW_CHARS: usize = 200;
/// Largest file read for a preview.
const PREVIEW_BYTES: u64 = 512 * 1024;

pub(crate) fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Read and change one root's state under one lock, so a resolve and a
/// cycle's record never overwrite each other's change.
pub(crate) static STATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Whether `rel` is still there (any kind of entry, not followed).
fn present(root: &Root, rel: &str) -> bool {
    fs::symlink_metadata(root.work_tree.join(rel)).is_ok()
}

/// Add `new` records to `saved` (one per file and copy) and drop those whose
/// copy is gone.
pub(crate) fn merge_records(root: &Root, saved: &mut SavedState, new: &[ClashRecord]) {
    for r in new {
        if !saved
            .clashes
            .iter()
            .any(|c| c.path == r.path && c.copy == r.copy)
        {
            saved.clashes.push(r.clone());
        }
    }
    saved.clashes.retain(|c| present(root, &c.copy));
    saved.resolved_copies.retain(|c| present(root, c));
}

/// Most kept-both copies remembered per root.
const RESOLVED_KEPT: usize = 500;

/// Copies the conflict copies item must not offer: every open clash's copy,
/// and the copies the person kept on purpose (Keep both, Done) while they
/// are still there.
pub(crate) fn skip_copies(root: &Root) -> Vec<String> {
    let saved = load_state(root);
    saved
        .clashes
        .iter()
        .map(|c| c.copy.clone())
        .chain(saved.resolved_copies.iter().cloned())
        .filter(|c| present(root, c))
        .collect()
}

/// The root's open clashes: the saved records whose copy is still there.
pub(crate) fn open(root: &Root) -> Vec<ClashRecord> {
    load_state(root)
        .clashes
        .into_iter()
        .filter(|c| present(root, &c.copy))
        .collect()
}

/// `rel`'s text for a card (no symlink followed anywhere on its path).
fn read_text(root: &Root, rel: &str) -> Result<String, String> {
    match roots::read_in_root(&root.work_tree, rel, PREVIEW_BYTES) {
        Ok(Some(text)) => Ok(text),
        Ok(None) => Err("missing".into()),
        Err(roots::PathError::TooLarge(_)) => Err("too large to show".into()),
        Err(e) => Err(e.to_string()),
    }
}

/// A side's preview: its first lines, each cut to [`PREVIEW_CHARS`].
pub(crate) fn side(text: Result<&str, &str>) -> ClashSide {
    match text {
        Ok(text) => ClashSide {
            lines: text
                .lines()
                .take(PREVIEW_LINES)
                .map(|l| {
                    if l.chars().count() > PREVIEW_CHARS {
                        let cut: String = l.chars().take(PREVIEW_CHARS).collect();
                        format!("{cut}…")
                    } else {
                        l.to_string()
                    }
                })
                .collect(),
            total_lines: text.lines().count() as u32,
            unreadable: None,
        },
        Err(why) => ClashSide {
            lines: Vec::new(),
            total_lines: 0,
            unreadable: Some(why.to_string()),
        },
    }
}

/// Lines only in `a`, and only in `b` (as sets of lines: a short summary,
/// not a diff).
pub(crate) fn only_counts(a: &str, b: &str) -> (u32, u32) {
    use std::collections::BTreeSet;
    let left: BTreeSet<&str> = a.lines().collect();
    let right: BTreeSet<&str> = b.lines().collect();
    (
        left.difference(&right).count() as u32,
        right.difference(&left).count() as u32,
    )
}

/// The card for `r` in `root`; `contents` false leaves out every line of
/// text (the agents' read).
pub(crate) fn card(root: &Root, r: &ClashRecord, contents: bool) -> SyncClash {
    // Only the person's card reads the files (the bell's query runs often).
    let changed = contents
        && match roots::resolve_in_root(&root.work_tree, &r.path) {
            Ok(full) => fs::read(full)
                .map(|b| hash(&b) != r.mine_hash)
                .unwrap_or(true),
            Err(_) => true,
        };
    let (mine, theirs, only_mine, only_theirs) = if contents {
        let mine_text = read_text(root, &r.path);
        let theirs_text = read_text(root, &r.copy);
        let (a, b) = match (&mine_text, &theirs_text) {
            (Ok(m), Ok(t)) => only_counts(m, t),
            _ => (0, 0),
        };
        (
            Some(side(mine_text.as_deref().map_err(String::as_str))),
            Some(side(theirs_text.as_deref().map_err(String::as_str))),
            a,
            b,
        )
    } else {
        (None, None, 0, 0)
    };
    SyncClash {
        root: root.id.clone(),
        path: r.path.clone(),
        copy: r.copy.clone(),
        mine_machine: r.mine_machine.clone(),
        theirs_machine: r.theirs_machine.clone(),
        at: r.at.clone(),
        replaceable: r.replaceable,
        editing: r.editing,
        changed: r.replaceable && changed,
        mine,
        theirs,
        only_mine,
        only_theirs,
    }
}

/// What a resolve did, for the card's note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ClashResolved {
    /// The file the person goes on with (Edit: the one to open).
    pub path: String,
    /// The sentence the card shows.
    pub text: String,
    /// The clash is still open (Edit).
    pub open: bool,
}

/// Refuse `rel` unless it is a safe work-tree path inside the root's tracked
/// folders.
fn check_path(root: &Root, rel: &str) -> Result<(), String> {
    roots::resolve_in_root(&root.work_tree, rel).map_err(|e| format!("{rel}: {e}"))?;
    if !root.allowed(rel) {
        return Err(format!("{rel} is outside what this root syncs."));
    }
    Ok(())
}

/// The person's `choice` on the clash at `path` (see the module notes).
/// `path` and `copy` name one record, exactly as the card shows them.
pub(crate) fn resolve(
    root: &Root,
    path: &str,
    copy: &str,
    choice: ClashChoice,
) -> Result<ClashResolved, String> {
    check_path(root, path)?;
    check_path(root, copy)?;
    let _held = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut saved = load_state(root);
    let Some(at) = saved
        .clashes
        .iter()
        .position(|c| c.path == path && c.copy == copy)
    else {
        return Err(format!("{path} has no open clash with {copy}."));
    };
    let rec = saved.clashes[at].clone();
    if !present(root, &rec.copy) {
        // The copy went another way (the other Mac, by hand): nothing to do.
        close(&mut saved, at, false);
        save_state(root, &saved);
        return Ok(ClashResolved {
            path: rec.path,
            text: format!("{} is already gone; the clash is closed.", rec.copy),
            open: false,
        });
    }
    check_path(root, &rec.copy)?;
    let vault = root.work_tree.as_path();
    let resolved = match choice {
        ClashChoice::KeepMine | ClashChoice::KeepTheirs if !rec.replaceable => {
            return Err(format!(
                "A folder now holds {}'s name, so one can't replace the other: keep both, or edit them by hand.",
                rec.path
            ))
        }
        ClashChoice::KeepMine => {
            let _lease =
                crate::automated_history::workspace_edit_lease(vault).map_err(String::from)?;
            {
                let _guard = crate::vault::lock_mutate();
                let now = fs::read(root.work_tree.join(&rec.copy))
                    .map_err(|e| format!("{}: {e}", rec.copy))?;
                copy_unchanged(&rec, &now)?;
                crate::vault::delete_note(vault, &rec.copy)?;
            }
            ClashResolved {
                text: format!(
                    "Kept this Mac's {}; {}'s version is in the vault's trash.",
                    rec.path, rec.theirs_machine
                ),
                path: rec.path.clone(),
                open: false,
            }
        }
        ClashChoice::KeepTheirs => {
            let _lease =
                crate::automated_history::workspace_edit_lease(vault).map_err(String::from)?;
            let want = rec.mine_hash.clone();
            let name = rec.path.clone();
            let check = rec.clone();
            crate::vault::replace_from(vault, &rec.copy, &rec.path, move |copy, file| {
                copy_unchanged(&check, copy)?;
                if hash(file) == want {
                    Ok(())
                } else {
                    Err(format!(
                        "{name} changed since the clash, so it wasn't replaced. Look again, or keep both."
                    ))
                }
            })?;
            ClashResolved {
                text: format!(
                    "Kept {}'s version as {}; this Mac's is in the vault's trash.",
                    rec.theirs_machine, rec.path
                ),
                path: rec.path.clone(),
                open: false,
            }
        }
        ClashChoice::KeepBoth => ClashResolved {
            text: format!("Kept both: {} and {}.", rec.path, rec.copy),
            path: rec.path.clone(),
            open: false,
        },
        ClashChoice::Done => ClashResolved {
            text: format!("Done with {}; the clash is closed.", rec.path),
            path: rec.path.clone(),
            open: false,
        },
        ClashChoice::Edit => {
            saved.clashes[at].editing = true;
            save_state(root, &saved);
            return Ok(ClashResolved {
                text: format!(
                    "Editing {}: merge what you need from {}, then press Done (or delete the copy).",
                    rec.path, rec.copy
                ),
                path: rec.path,
                open: true,
            });
        }
    };
    let kept = matches!(choice, ClashChoice::KeepBoth | ClashChoice::Done);
    close(&mut saved, at, kept);
    save_state(root, &saved);
    Ok(resolved)
}

/// Refuse when the copy no longer holds the other Mac's version as the
/// merge wrote it (someone edited it since): the person looks again.
fn copy_unchanged(rec: &ClashRecord, now: &[u8]) -> Result<(), String> {
    if hash(now) == rec.theirs_hash {
        Ok(())
    } else {
        Err(format!(
            "{} changed since the clash, so nothing was moved. Look again, or keep both.",
            rec.copy
        ))
    }
}

/// Drop record `at` and its copy from the clash state; `kept` remembers the
/// copy as one the person kept on purpose, so it is never offered again as
/// a conflict copy.
fn close(saved: &mut SavedState, at: usize, kept: bool) {
    let rec = saved.clashes.remove(at);
    saved.clash_copies.retain(|c| *c != rec.copy);
    if kept && !saved.resolved_copies.contains(&rec.copy) {
        saved.resolved_copies.push(rec.copy);
        let over = saved.resolved_copies.len().saturating_sub(RESOLVED_KEPT);
        saved.resolved_copies.drain(..over);
    }
}
