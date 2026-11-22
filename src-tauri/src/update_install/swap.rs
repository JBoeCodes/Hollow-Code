//! The moves that put the new app in place, journaled one by one, and
//! putting them back.
//!
//! An install over an existing app is planned as
//!
//! 1. `swap staging <-> installed`: one `renamex_np(RENAME_SWAP)`, so the
//!    install path always holds an app (the new one after it, the old one
//!    before it), and the old app is now at `staging`;
//! 2. `rename previous -> discard` (when there is a previous and the old app
//!    verified, so it is worth keeping);
//! 3. `rename staging -> previous` (or `-> discard` when the old app did not
//!    verify: it never replaces a verified previous).
//!
//! A fresh install is the single `rename staging -> installed`. A go back is
//! the single `swap previous <-> installed`, after the Sessions restore's
//! file renames when it restores data (`restore.rs`).
//!
//! Each move's journal entry is written before the moves start and
//! `movesDone` after each one. A crash between a move and that write is
//! found by inode: a folder keeps its inode through a rename, and the plan
//! records the inode of the folder each move carries to its `to`.

use super::effects::Fs;
use super::journal::{Journal, Move, Op};
use super::verify;
use super::Layout;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub(crate) fn ino(path: &Path) -> Option<u64> {
    std::fs::symlink_metadata(path).ok().map(|m| m.ino())
}

/// The install's plan, or an error naming a path in the way.
pub(crate) fn plan_install(
    layout: &Layout,
    new_ino: u64,
    keep_current: bool,
) -> Result<Vec<Move>, String> {
    if layout.discard.exists() {
        return Err(format!(
            "{} is left from an earlier install",
            layout.discard.display()
        ));
    }
    let Some(old_ino) = ino(&layout.installed) else {
        return Ok(vec![Move {
            op: Op::Rename,
            from: layout.staging.clone(),
            to: layout.installed.clone(),
            ino: new_ino,
        }]);
    };
    let mut moves = vec![Move {
        op: Op::Swap,
        from: layout.staging.clone(),
        to: layout.installed.clone(),
        ino: new_ino,
    }];
    if keep_current {
        if let Some(prev_ino) = ino(&layout.previous) {
            moves.push(Move {
                op: Op::Rename,
                from: layout.previous.clone(),
                to: layout.discard.clone(),
                ino: prev_ino,
            });
        }
        moves.push(Move {
            op: Op::Rename,
            from: layout.staging.clone(),
            to: layout.previous.clone(),
            ino: old_ino,
        });
    } else {
        moves.push(Move {
            op: Op::Rename,
            from: layout.staging.clone(),
            to: layout.discard.clone(),
            ino: old_ino,
        });
    }
    Ok(moves)
}

pub(crate) fn plan_go_back(layout: &Layout) -> Result<Vec<Move>, String> {
    let prev_ino = ino(&layout.previous).ok_or("there is no previous app")?;
    Ok(vec![Move {
        op: Op::Swap,
        from: layout.previous.clone(),
        to: layout.installed.clone(),
        ino: prev_ino,
    }])
}

/// Whether `m` has happened: its folder is at `to`.
fn done(m: &Move) -> bool {
    ino(&m.to) == Some(m.ino)
}

/// Whether `m` has not happened (or was undone): its folder is at `from`
/// and, for a rename, nothing is at `to`.
fn not_done(m: &Move) -> bool {
    ino(&m.from) == Some(m.ino) && (m.op == Op::Swap || !m.to.exists())
}

fn apply(fs: &dyn Fs, m: &Move) -> std::io::Result<()> {
    match m.op {
        Op::Rename => {
            if m.to.exists() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("{} is in the way", m.to.display()),
                ));
            }
            fs.rename(&m.from, &m.to)
        }
        Op::Swap => fs.swap(&m.from, &m.to),
    }
}

fn reverse(fs: &dyn Fs, m: &Move) -> std::io::Result<()> {
    match m.op {
        Op::Rename => fs.rename(&m.to, &m.from),
        Op::Swap => fs.swap(&m.from, &m.to),
    }
}

/// Run the journal's plan from `movesDone`, saving after each move.
pub(crate) fn run_moves(fs: &dyn Fs, journal: &mut Journal) -> Result<(), String> {
    let all = journal.record.plan.as_ref().map_or(0, Vec::len);
    run_moves_until(fs, journal, all)
}

/// Run the plan's moves from `movesDone` up to (not including) move `end`
/// (a go back checks its restored data between its restore moves and the
/// swap; the whole plan is in the journal first, so a crash in between is
/// undone, never kept half done).
pub(crate) fn run_moves_until(
    fs: &dyn Fs,
    journal: &mut Journal,
    end: usize,
) -> Result<(), String> {
    let moves = journal.record.plan.clone().unwrap_or_default();
    let end = end.min(moves.len());
    while journal.record.moves_done < end {
        let Some(m) = moves.get(journal.record.moves_done) else {
            return Err("the journal's move count is past its plan".into());
        };
        apply(fs, m).map_err(|e| {
            format!(
                "could not move {} to {}: {e}",
                m.from.display(),
                m.to.display()
            )
        })?;
        journal.record.moves_done += 1;
        journal
            .save()
            .map_err(|e| format!("could not write the journal: {e}"))?;
    }
    Ok(())
}

/// Count a move that happened before the crash but was never recorded.
fn catch_up(journal: &mut Journal) {
    let moves = journal.record.plan.clone().unwrap_or_default();
    if let Some(m) = moves.get(journal.record.moves_done) {
        if done(m) {
            journal.record.moves_done += 1;
            let _ = journal.save();
        }
    }
}

/// Undo the recorded moves, newest first. A move already undone (a crash
/// between the undo and its journal write) is skipped; anything else stops
/// with the paths for a person.
pub(crate) fn undo_moves(fs: &dyn Fs, journal: &mut Journal) -> Result<(), String> {
    let moves = journal.record.plan.clone().unwrap_or_default();
    while journal.record.moves_done > 0 {
        let k = journal.record.moves_done - 1;
        let Some(m) = moves.get(k) else {
            return Err(format!(
                "the journal records {} moves done but plans {}; resolve by hand",
                journal.record.moves_done,
                moves.len()
            ));
        };
        if done(m) {
            reverse(fs, m).map_err(|e| {
                format!(
                    "could not move {} back to {}: {e}",
                    m.to.display(),
                    m.from.display()
                )
            })?;
        } else if !not_done(m) {
            return Err(format!(
                "could not undo {} -> {}: neither path holds the folder the journal expects; resolve by hand",
                m.from.display(),
                m.to.display()
            ));
        }
        journal.record.moves_done = k;
        let _ = journal.save();
    }
    Ok(())
}

/// How recovery settled one unfinished journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Recovered {
    /// Nothing had moved: the staging copy removed, journal `failed`.
    NothingMoved,
    /// Some or all moves undone: journal `rolled-back`.
    Undone,
    /// Every move was done and the app at the install path verifies: it
    /// stands, journal `healthy` with `confirmed: false`.
    Kept,
}

/// Finish or undo one unfinished journal (`prepared` through `started`, or
/// `rolling-back`). Never deletes a kept app unless the installed one
/// verifies in its place.
pub(crate) fn recover(
    fs: &dyn Fs,
    signer: &dyn super::effects::Signer,
    layout: &Layout,
    journal: &mut Journal,
    now: &str,
) -> Result<Recovered, String> {
    let record_err = |e: std::io::Error| format!("could not write the journal: {e}");
    // Stopped after its last reopen attempt failed (between tries, or stuck
    // with a process it would not kill): go back rather than keep an app
    // that was not seen to start. A last attempt that was healthy keeps it.
    let rolling_back = journal.record.state == "rolling-back"
        || (journal.record.kind == super::journal::Kind::Install
            && last_attempt_failed(&journal.record));
    if journal.record.state != "rolling-back" {
        catch_up(journal);
    }
    let plan = journal.record.plan.clone().unwrap_or_default();
    if !rolling_back && journal.record.moves_done == 0 {
        // A go back's staging is not its own; an install's holds only the
        // new copy until the first move.
        if journal.record.kind == super::journal::Kind::Install {
            fs.remove_all(&layout.staging)
                .map_err(|e| format!("could not remove {}: {e}", layout.staging.display()))?;
        }
        journal
            .step(
                "failed",
                now,
                serde_json::json!({ "recovered": true, "reason": "stopped before anything was moved" }),
            )
            .map_err(record_err)?;
        return Ok(Recovered::NothingMoved);
    }
    if !rolling_back
        && journal.record.moves_done == plan.len()
        && verify::check_bundle(&layout.installed, signer, None, None).is_ok()
    {
        fs.remove_all(&layout.discard)
            .map_err(|e| format!("could not remove {}: {e}", layout.discard.display()))?;
        journal
                .step(
                    "healthy",
                    now,
                    serde_json::json!({
                        "recovered": true,
                        "confirmed": false,
                        "reason": "the installer stopped after the swap; the app in place verifies, so it stands"
                    }),
                )
                .map_err(record_err)?;
        return Ok(Recovered::Kept);
    }
    journal.record.state = "rolling-back".into();
    let _ = journal.save();
    undo_moves(fs, journal)?;
    if journal.record.kind == super::journal::Kind::Install {
        fs.remove_all(&layout.staging)
            .map_err(|e| format!("could not remove {}: {e}", layout.staging.display()))?;
    }
    journal
        .step(
            "rolled-back",
            now,
            serde_json::json!({ "recovered": true, "reason": "the installer stopped part way; its moves were undone" }),
        )
        .map_err(record_err)?;
    Ok(Recovered::Undone)
}

/// Whether the journal's last reopen attempt was not healthy.
pub(crate) fn last_attempt_failed(record: &super::journal::Record) -> bool {
    record
        .attempts
        .last()
        .is_some_and(|a| a.outcome != "healthy")
}
