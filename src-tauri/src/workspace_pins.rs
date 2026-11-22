//! Pinned workspaces (2026-10-21): the one pin state the sidebar's Pinned
//! group, Home and the Main agent's `update_workspace` all read and write.
//!
//! A pin is `pinned: <rank>` in the workspace's brief (WORKSPACE.md
//! frontmatter), lower ranks first, ties by id. It lives with the workspace,
//! so it survives restarts and goes with the vault, and a merge or an
//! archive clears it in the same write that changes the status. Every write
//! is Hollow's compare-and-swap on the brief ([`crate::shared_docs::update_doc`]),
//! so a pin made while a chat in that workspace edits its brief never undoes
//! the chat's edit (or the other way round), and the brief's mtime is kept
//! unless a chat runs there, so pinning doesn't reorder the recent lists.
//!
//! Tracker-managed workspaces, legacy `notion-*` ones and connected folders
//! can be pinned too: like `canvas_theme` it is a local display preference.
//! A sync patches only its own keys in a brief, so a pin never reaches the
//! tracker. Built-in workspaces and archived ones can't be pinned;
//! [`crate::workspaces::summary`] ignores a stale pin left in an archived
//! brief (archived by a sync or by hand).

use crate::frontmatter::{self, KeyEdit};
use crate::workspaces::{self, Workspace};
use std::path::Path;

/// The most pins a position can name; past the last pin is the end.
pub(crate) const MAX_POSITION: u32 = 1000;

/// What to do with a workspace's pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinChange {
    /// Pin it at the end of the pinned list (already pinned: no change).
    Pin,
    /// Unpin it (not pinned: no change).
    Unpin,
    /// Pin it, or move its pin, to this 1-based position among the pins.
    MoveTo(u32),
}

/// The pinned workspaces' ids, in pin order.
#[cfg(test)]
fn pinned_ids(root: &Path) -> Result<Vec<String>, String> {
    Ok(order(&workspaces::list(root)?))
}

fn order(all: &[Workspace]) -> Vec<String> {
    let mut pins: Vec<(u32, &str)> = all
        .iter()
        .filter_map(|w| w.pinned.map(|rank| (rank, w.id.as_str())))
        .collect();
    pins.sort();
    pins.into_iter().map(|(_, id)| id.to_string()).collect()
}

/// Apply `change` to workspace `id` and return its summary. A move rewrites
/// only the briefs whose rank changes.
pub fn set_pin(root: &Path, id: &str, change: PinChange) -> Result<Workspace, String> {
    let target = workspaces::summary(root, id)?;
    if workspaces::is_builtin(id) {
        return Err(format!(
            "{} is built in and always has its own place; it can't be pinned.",
            target.title
        ));
    }
    let all = workspaces::list(root)?;
    let mut ids = order(&all);
    let at = ids.iter().position(|p| p == id);
    match change {
        PinChange::Unpin => {
            if at.is_some() || raw_pin(root, id)? {
                write_rank(root, id, None, true)?;
            }
        }
        PinChange::Pin => {
            refuse_archived(&target)?;
            if at.is_none() {
                let next = all.iter().filter_map(|w| w.pinned).max().unwrap_or(0) + 1;
                write_rank(root, id, Some(next), true)?;
            }
        }
        PinChange::MoveTo(position) => {
            refuse_archived(&target)?;
            if let Some(i) = at {
                ids.remove(i);
            }
            let slot = (position.max(1) as usize - 1).min(ids.len());
            ids.insert(slot, id.to_string());
            let rank = |wid: &str| all.iter().find(|w| w.id == wid).and_then(|w| w.pinned);
            for (i, wid) in ids.iter().enumerate() {
                let want = i as u32 + 1;
                if rank(wid) != Some(want) {
                    write_rank(root, wid, Some(want), wid == id)?;
                }
            }
        }
    }
    workspaces::summary(root, id)
}

fn refuse_archived(w: &Workspace) -> Result<(), String> {
    if workspaces::is_archived_status(&w.status) {
        return Err(format!(
            "{} is archived, and archived workspaces can't be pinned. Unarchive it first.",
            w.title
        ));
    }
    Ok(())
}

/// Whether the brief has a `pinned` key at all (a stale one in an archived
/// brief still gets removed by Unpin).
fn raw_pin(root: &Path, id: &str) -> Result<bool, String> {
    let text = workspaces::read_file(root, &format!("workspaces/{id}/WORKSPACE.md"))?.content;
    Ok(frontmatter::parse_map(&text).0.contains_key("pinned"))
}

/// Set (or with `None` remove) `pinned` in workspace `id`'s brief,
/// compare-and-swap. A brief archived since it was read is unpinned instead;
/// for the workspace being pinned (`target`) that is refused, while another
/// one a move renumbers just drops out of the pins.
fn write_rank(root: &Path, id: &str, rank: Option<u32>, target: bool) -> Result<(), String> {
    crate::shared_docs::update_doc(root, id, "WORKSPACE.md", |text| {
        let archived = frontmatter::parse_map(text)
            .0
            .get("status")
            .is_some_and(|s| workspaces::is_archived_status(s));
        if archived && rank.is_some() && target {
            return Err("This workspace was archived meanwhile, so it wasn't pinned.".into());
        }
        let edit = match rank {
            Some(r) if !archived => KeyEdit::Set(r.to_string()),
            _ => KeyEdit::Remove,
        };
        frontmatter::patch(text, &[("pinned", edit)], None)
    })
    .map(|_| ())
}

/// `update_workspace`'s pin arguments: `pinned` true pins (at the end, or at
/// `position`), false unpins; `position` alone pins or moves the pin there.
pub fn change_from_args(
    pinned: Option<bool>,
    position: Option<u32>,
) -> Result<Option<PinChange>, String> {
    match (pinned, position) {
        (Some(false), Some(_)) => Err("pinPosition places a pin; leave it out to unpin".into()),
        (_, Some(0)) => Err("pinPosition counts from 1 (the top)".into()),
        (_, Some(p)) => Ok(Some(PinChange::MoveTo(p.min(MAX_POSITION)))),
        (Some(true), None) => Ok(Some(PinChange::Pin)),
        (Some(false), None) => Ok(Some(PinChange::Unpin)),
        (None, None) => Ok(None),
    }
}

/// Rename, set the status and/or change the pin of workspace `id`
/// (`update_workspace`, from the app and from agents). The title and status
/// go first, so archiving and pinning in one call is refused.
pub fn update(
    root: &Path,
    id: &str,
    title: Option<&str>,
    status: Option<&str>,
    pinned: Option<bool>,
    position: Option<u32>,
) -> Result<Workspace, String> {
    let change = change_from_args(pinned, position)?;
    // Refused before anything is written, so "archived and pinned" never
    // leaves a half-done change.
    if status.is_some_and(workspaces::is_archived_status)
        && matches!(change, Some(PinChange::Pin | PinChange::MoveTo(_)))
    {
        return Err("Archived workspaces can't be pinned: archive or pin, not both.".into());
    }
    let mut out = None;
    if title.is_some() || status.is_some() || change.is_none() {
        out = Some(workspaces::update_meta(root, id, title, status)?);
    }
    match change {
        Some(change) => set_pin(root, id, change),
        None => Ok(out.expect("update_meta ran")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn brief(root: &Path, id: &str) -> String {
        fs::read_to_string(root.join(format!("workspaces/{id}/WORKSPACE.md"))).unwrap()
    }

    #[test]
    fn pins_keep_their_order_and_a_move_rewrites_only_changed_briefs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let a = workspaces::create(root, "Alpha", "A").unwrap().id;
        let b = workspaces::create(root, "Beta", "B").unwrap().id;
        let c = workspaces::create(root, "Gamma", "C").unwrap().id;
        assert!(pinned_ids(root).unwrap().is_empty());
        let mtime = fs::metadata(root.join(format!("workspaces/{a}/WORKSPACE.md")))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(set_pin(root, &a, PinChange::Pin).unwrap().pinned, Some(1));
        set_pin(root, &b, PinChange::Pin).unwrap();
        set_pin(root, &c, PinChange::Pin).unwrap();
        // Pinning again changes nothing.
        set_pin(root, &a, PinChange::Pin).unwrap();
        assert_eq!(
            pinned_ids(root).unwrap(),
            [a.as_str(), b.as_str(), c.as_str()]
        );
        // The mtime is kept (no chat runs there), so recent lists don't jump.
        assert_eq!(
            fs::metadata(root.join(format!("workspaces/{a}/WORKSPACE.md")))
                .unwrap()
                .modified()
                .unwrap(),
            mtime
        );
        // Move Gamma to the top: every rank changes.
        set_pin(root, &c, PinChange::MoveTo(1)).unwrap();
        assert_eq!(
            pinned_ids(root).unwrap(),
            [c.as_str(), a.as_str(), b.as_str()]
        );
        // Move Beta up one: only Alpha and Beta are rewritten.
        let gamma = brief(root, &c);
        set_pin(root, &b, PinChange::MoveTo(2)).unwrap();
        assert_eq!(
            pinned_ids(root).unwrap(),
            [c.as_str(), b.as_str(), a.as_str()]
        );
        assert_eq!(brief(root, &c), gamma);
        // Past the end is the end.
        set_pin(root, &c, PinChange::MoveTo(99)).unwrap();
        assert_eq!(
            pinned_ids(root).unwrap(),
            [b.as_str(), a.as_str(), c.as_str()]
        );
        // Unpinning removes the key and leaves the others in order.
        set_pin(root, &a, PinChange::Unpin).unwrap();
        assert!(!brief(root, &a).contains("pinned:"));
        assert_eq!(pinned_ids(root).unwrap(), [b.as_str(), c.as_str()]);
        // Unpinning an unpinned workspace is no change.
        let before = brief(root, &a);
        set_pin(root, &a, PinChange::Unpin).unwrap();
        assert_eq!(brief(root, &a), before);
        // MoveTo pins an unpinned workspace at that place.
        set_pin(root, &a, PinChange::MoveTo(2)).unwrap();
        assert_eq!(
            pinned_ids(root).unwrap(),
            [b.as_str(), a.as_str(), c.as_str()]
        );
        // The body and the other keys are untouched.
        let (fm, body) = frontmatter::parse_map(&brief(root, &a));
        assert_eq!(fm.get("title").map(String::as_str), Some("Alpha"));
        assert!(body.contains("A"));
    }

    #[test]
    fn archived_and_built_in_workspaces_are_never_pinned_and_archiving_unpins() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let a = workspaces::create(root, "Alpha", "A").unwrap().id;
        set_pin(root, &a, PinChange::Pin).unwrap();
        // Archiving through update_meta (the menu, the gardener, agents) unpins.
        let archived = workspaces::update_meta(root, &a, None, Some("archived")).unwrap();
        assert_eq!(archived.pinned, None);
        assert!(!brief(root, &a).contains("pinned:"));
        let err = set_pin(root, &a, PinChange::Pin).unwrap_err();
        assert!(err.contains("archived"), "{err}");
        assert!(set_pin(root, &a, PinChange::MoveTo(1)).is_err());
        // Unarchiving doesn't bring the pin back.
        let back = workspaces::update_meta(root, &a, None, Some("active")).unwrap();
        assert_eq!(back.pinned, None);
        // A stale pin in an archived brief (a sync, an edit by hand) doesn't count.
        let path = root.join(format!("workspaces/{a}/WORKSPACE.md"));
        let text = fs::read_to_string(&path).unwrap();
        fs::write(
            &path,
            text.replacen("status: active", "status: archived\npinned: 1", 1),
        )
        .unwrap();
        assert_eq!(workspaces::summary(root, &a).unwrap().pinned, None);
        assert!(pinned_ids(root).unwrap().is_empty());
        // Unpin still clears it.
        set_pin(root, &a, PinChange::Unpin).unwrap();
        assert!(!brief(root, &a).contains("pinned:"));
        // Built-ins refuse.
        workspaces::ensure_quick_chats(root).unwrap();
        assert!(set_pin(root, workspaces::QUICK_CHATS_ID, PinChange::Pin).is_err());
    }

    #[test]
    fn a_notion_mirror_can_be_pinned_and_sync_keeps_the_pin() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let id = "notion-00000000000000000000000000000002";
        workspaces::ensure_named(root, id, "Mirror", "From Notion").unwrap();
        assert_eq!(set_pin(root, id, PinChange::Pin).unwrap().pinned, Some(1));
        // Notion sync patches only its own keys in the brief.
        let text = brief(root, id);
        let synced = frontmatter::patch(
            &text,
            &[
                ("title", KeyEdit::Set("Renamed in Notion".into())),
                ("status", KeyEdit::Set("active".into())),
            ],
            None,
        )
        .unwrap();
        assert!(synced.contains("pinned: 1"));
    }

    #[test]
    fn update_takes_title_status_and_pin_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let a = workspaces::create(root, "Alpha", "A").unwrap().id;
        let b = workspaces::create(root, "Beta", "B").unwrap().id;
        assert_eq!(
            update(root, &a, None, None, Some(true), None)
                .unwrap()
                .pinned,
            Some(1)
        );
        update(root, &b, Some("Beta two"), None, None, Some(1)).unwrap();
        assert_eq!(pinned_ids(root).unwrap(), [b.as_str(), a.as_str()]);
        assert_eq!(workspaces::summary(root, &b).unwrap().title, "Beta two");
        assert!(update(root, &a, None, None, Some(false), Some(1)).is_err());
        assert!(update(root, &a, None, None, None, Some(0)).is_err());
        // Archive and pin in one call: refused before anything is written.
        assert!(update(root, &a, None, Some("archived"), Some(true), None).is_err());
        assert_eq!(workspaces::summary(root, &a).unwrap().status, "active");
        assert_eq!(pinned_ids(root).unwrap(), [b.as_str(), a.as_str()]);
        update(root, &a, None, Some("archived"), None, None).unwrap();
        assert_eq!(pinned_ids(root).unwrap(), [b.as_str()]);
        update(root, &b, None, None, Some(false), None).unwrap();
        assert!(pinned_ids(root).unwrap().is_empty());
        // A legacy Notion workspace is a plain one since 2026-11-10: its pin,
        // title and status all change here.
        let n = "notion-00000000000000000000000000000002";
        workspaces::ensure_named(root, n, "Mirror", "From Notion").unwrap();
        assert!(update(root, n, None, None, Some(true), None).is_ok());
        assert!(update(root, n, Some("x"), None, Some(true), None).is_ok());
    }

    #[test]
    fn a_merge_unpins_the_workspace_it_archives() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let a = workspaces::create(root, "Alpha", "A").unwrap().id;
        let b = workspaces::create(root, "Beta", "B").unwrap().id;
        set_pin(root, &a, PinChange::Pin).unwrap();
        set_pin(root, &b, PinChange::Pin).unwrap();
        let plan = crate::workspace_merge::plan(root, &a, &b).unwrap();
        crate::workspace_merge::merge(root, &a, &b, &plan.hash, None).unwrap();
        assert!(!brief(root, &a).contains("pinned:"));
        assert_eq!(pinned_ids(root).unwrap(), [b.as_str()]);
    }
}
