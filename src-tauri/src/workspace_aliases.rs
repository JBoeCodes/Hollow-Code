//! Workspace id aliases: which `local-*` workspace id a moved project had
//! before (Move Hollow home, 2026-11-13).
//!
//! A connected project's id is `local-` + sha256(its absolute path)[..24]
//! (`directory_connections::project`), so moving the folder gives it a new
//! id. Move Hollow home rewrites every mutable id column in the Sessions
//! database, but `audit_log` is append-only and hash-chained (`audit`): its
//! rows keep the id they were written with, forever. This table records each
//! old id and the id it became, so every reader that looks up audit, usage
//! or history by workspace id can find the rows written before a move.
//!
//! Storage: one table in the Sessions database, `workspace_aliases`, one row
//! per (old id, move). Rows are append-only (`BEFORE UPDATE` and
//! `BEFORE DELETE` triggers abort, as for `audit_log`); a move's rollback
//! restores the whole database file, so it never deletes a row. The move
//! writes the rows into its rewritten copy of the database inside the same
//! transaction as the re-key, so the aliases exist exactly when the re-key
//! does.
//!
//! Lookup: [`Aliases::load`] reads the whole table (one row per moved
//! project, a handful) and builds the forward map old to new. When an old id
//! was recorded more than once (moved, rolled back, moved again elsewhere),
//! the newest row wins.
//! - [`Aliases::current`] follows the chain to the id the workspace has now
//!   (A moved to B, then B to C: A, B and C all give C). A cycle stops at the
//!   id it started from.
//! - [`Aliases::ids_for`] is every id that names the same workspace now: the
//!   current id and every id whose chain ends there. A reader filtering by a
//!   workspace id asks with `IN (ids_for(id))`, so an old id finds the new
//!   rows and the new id finds the old rows.
//!
//! Known limit: ids are hashes of paths, so one id can name two projects in
//! turn. If one move takes project A to the path project B had while B moves
//! on (A to B, B to C), A resolves to C. The move's plan blocks that when B
//! has a workspace folder (`idCollision`); without one, B has no history.
//!
//! A database without the table (older, or a test fixture) has no aliases:
//! both lookups then return the id itself.

use rusqlite::{params, Connection};
use std::collections::BTreeMap;

/// The table, its index and the append-only triggers. `session_store`'s
/// `apply_schema` runs it; Move Hollow home runs it on its copy too.
pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS workspace_aliases( \
   old_id TEXT NOT NULL, \
   new_id TEXT NOT NULL, \
   move_id TEXT NOT NULL, \
   at TEXT NOT NULL, \
   PRIMARY KEY(old_id, move_id) \
 ); \
 CREATE INDEX IF NOT EXISTS idx_workspace_aliases_new ON workspace_aliases(new_id); \
 CREATE TRIGGER IF NOT EXISTS workspace_aliases_no_update BEFORE UPDATE ON workspace_aliases \
   BEGIN SELECT RAISE(ABORT, 'workspace_aliases is append-only'); END; \
 CREATE TRIGGER IF NOT EXISTS workspace_aliases_no_delete BEFORE DELETE ON workspace_aliases \
   BEGIN SELECT RAISE(ABORT, 'workspace_aliases is append-only'); END;";

/// How many links [`Aliases::current`] follows at most.
const MAX_HOPS: usize = 64;

/// The old-to-new workspace id map, loaded once per read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Aliases {
    forward: BTreeMap<String, String>,
}

impl Aliases {
    /// Every alias row, the newest per old id. No table: none.
    pub(crate) fn load(conn: &Connection) -> Aliases {
        let mut forward = BTreeMap::new();
        let Ok(mut stmt) =
            conn.prepare("SELECT old_id, new_id FROM workspace_aliases ORDER BY rowid ASC")
        else {
            return Aliases::default();
        };
        let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        else {
            return Aliases::default();
        };
        for (old, new) in rows.flatten() {
            if old != new {
                // `new` is a live id again from this move on (a project moved
                // back to where it was): it no longer points anywhere. This
                // also means the map never holds a cycle.
                forward.remove(&new);
                forward.insert(old, new);
            }
        }
        Aliases { forward }
    }

    /// Built from pairs (tests and the move's own checks).
    #[cfg(test)]
    pub(crate) fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Aliases {
        Aliases {
            forward: pairs
                .into_iter()
                .filter(|(a, b)| a != b)
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.forward.is_empty()
    }

    /// The id the workspace named `id` has now. A cycle (never written by
    /// [`Aliases::load`], possible from [`Aliases::from_pairs`]) stops at the
    /// first id seen twice.
    pub(crate) fn current(&self, id: &str) -> String {
        let mut cur = id;
        let mut seen = std::collections::BTreeSet::from([id]);
        for _ in 0..MAX_HOPS {
            match self.forward.get(cur) {
                Some(next) if seen.insert(next.as_str()) => cur = next,
                _ => break,
            }
        }
        cur.to_string()
    }

    /// The path a vault-relative `workspaces/<old id>/...` path has now, when
    /// its workspace moved; `None` otherwise.
    pub(crate) fn current_vault_path(&self, path: &str) -> Option<String> {
        let (id, rest) = split_workspace(path)?;
        let now = self.moved(id)?;
        Some(if rest.is_empty() {
            format!("workspaces/{now}")
        } else {
            format!("workspaces/{now}/{rest}")
        })
    }

    /// Every id that names the same workspace as `id` now, the current id
    /// first, then the older ones in order.
    pub(crate) fn ids_for(&self, id: &str) -> Vec<String> {
        let now = self.current(id);
        let mut out = vec![now.clone()];
        for old in self.forward.keys() {
            if *old != now && self.current(old) == now {
                out.push(old.clone());
            }
        }
        if !out.iter().any(|x| x == id) {
            out.push(id.to_string());
        }
        out
    }

    /// `Some(current id)` when `id` was moved, else `None`.
    pub(crate) fn moved(&self, id: &str) -> Option<String> {
        let now = self.current(id);
        (now != id).then_some(now)
    }

    /// Whether two vault-relative paths name the same file: equal, or both
    /// under `workspaces/<id>/` with ids that are the same workspace now and
    /// the same rest. History (History & recovery) records the path a file
    /// had when a run changed it; a moved project's folder is renamed to its
    /// new id.
    pub(crate) fn same_vault_path(&self, a: &str, b: &str) -> bool {
        if a == b {
            return true;
        }
        if self.is_empty() {
            return false;
        }
        match (split_workspace(a), split_workspace(b)) {
            (Some((ia, ra)), Some((ib, rb))) => {
                ra == rb && ia != ib && self.current(ia) == self.current(ib)
            }
            _ => false,
        }
    }

    /// The aliases of the vault's Sessions database, read only when the
    /// database exists (never creates one).
    pub(crate) fn read(vault: &std::path::Path) -> Aliases {
        let exists = crate::session_store::db_path_for(vault).is_some_and(|p| p.exists());
        if !exists {
            return Aliases::default();
        }
        crate::session_store::with_read(vault, |conn| Ok(Aliases::load(conn))).unwrap_or_default()
    }
}

/// `workspaces/<id>/<rest>` as (id, rest); `workspaces/<id>` as (id, "").
fn split_workspace(p: &str) -> Option<(&str, &str)> {
    let rest = p.strip_prefix("workspaces/")?;
    Some(rest.split_once('/').unwrap_or((rest, "")))
}

/// Record a move's pairs (old id, new id). Pairs whose ids are equal are
/// skipped; recording the same (old id, move) twice keeps the first row.
/// Returns how many rows were added.
pub(crate) fn record(
    conn: &Connection,
    pairs: &BTreeMap<String, String>,
    move_id: &str,
    at: &str,
) -> Result<u64, String> {
    conn.execute_batch(SCHEMA)
        .map_err(|e| format!("workspace aliases schema: {e}"))?;
    let mut n = 0;
    for (old, new) in pairs {
        if old == new {
            continue;
        }
        n += conn
            .execute(
                "INSERT OR IGNORE INTO workspace_aliases(old_id, new_id, move_id, at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![old, new, move_id, at],
            )
            .map_err(|e| format!("workspace aliases: {e}"))? as u64;
    }
    Ok(n)
}

/// `?, ?, ...` for `n` ids (an `IN (...)` list).
pub(crate) fn placeholders(n: usize) -> String {
    vec!["?"; n.max(1)].join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    #[test]
    fn workspace_aliases_follow_chains_both_ways() {
        let conn = db();
        let pairs = |p: &[(&str, &str)]| {
            p.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(
            record(&conn, &pairs(&[("A", "B"), ("X", "X")]), "m1", "t").unwrap(),
            1
        );
        assert_eq!(record(&conn, &pairs(&[("B", "C")]), "m2", "t").unwrap(), 1);
        let a = Aliases::load(&conn);
        assert_eq!(a.current("A"), "C");
        assert_eq!(a.current("B"), "C");
        assert_eq!(a.current("C"), "C");
        assert_eq!(a.current("Q"), "Q");
        assert_eq!(a.ids_for("C"), vec!["C", "A", "B"]);
        assert_eq!(a.ids_for("A"), vec!["C", "A", "B"]);
        assert_eq!(a.ids_for("Q"), vec!["Q"]);
        assert_eq!(a.moved("A").as_deref(), Some("C"));
        assert_eq!(a.moved("C"), None);
    }

    #[test]
    fn workspace_aliases_are_append_only_and_cycle_safe() {
        let conn = db();
        let mut p = BTreeMap::new();
        p.insert("A".to_string(), "B".to_string());
        record(&conn, &p, "m1", "t").unwrap();
        assert!(conn
            .execute("UPDATE workspace_aliases SET new_id = 'Z'", [])
            .unwrap_err()
            .to_string()
            .contains("append-only"));
        assert!(conn
            .execute("DELETE FROM workspace_aliases", [])
            .unwrap_err()
            .to_string()
            .contains("append-only"));
        // Same (old, move) again: kept once.
        assert_eq!(record(&conn, &p, "m1", "t").unwrap(), 0);
        // A project moved back where it was: its first id is live again.
        let mut back = BTreeMap::new();
        back.insert("B".to_string(), "A".to_string());
        record(&conn, &back, "m2", "t").unwrap();
        let a = Aliases::load(&conn);
        assert_eq!(a.current("A"), "A");
        assert_eq!(a.current("B"), "A");
        assert_eq!(a.ids_for("A"), vec!["A", "B"]);
        assert_eq!(a.moved("A"), None);
        // And on to a third place, then back to the second.
        let mut on = BTreeMap::new();
        on.insert("A".to_string(), "C".to_string());
        record(&conn, &on, "m3", "t").unwrap();
        let mut back2 = BTreeMap::new();
        back2.insert("C".to_string(), "B".to_string());
        record(&conn, &back2, "m4", "t").unwrap();
        let a = Aliases::load(&conn);
        for id in ["A", "B", "C"] {
            assert_eq!(a.current(id), "B", "{id}");
        }
        // A hand-built cycle stops instead of looping.
        let c = Aliases::from_pairs([("X", "Y"), ("Y", "Z"), ("Z", "Y")]);
        assert_eq!(c.current("X"), "Z");
    }

    #[test]
    fn workspace_aliases_match_vault_paths_across_a_move() {
        let a = Aliases::from_pairs([("local-old", "local-new")]);
        assert!(a.same_vault_path(
            "workspaces/local-old/WORKSPACE.md",
            "workspaces/local-new/WORKSPACE.md"
        ));
        assert!(a.same_vault_path("workspaces/local-new/x", "workspaces/local-old/x"));
        assert!(!a.same_vault_path("workspaces/local-old/a", "workspaces/local-new/b"));
        assert!(!a.same_vault_path(
            "workspaces/local-other/WORKSPACE.md",
            "workspaces/local-new/WORKSPACE.md"
        ));
        assert!(!a.same_vault_path("notes/local-old/x", "notes/local-new/x"));
        assert_eq!(
            a.current_vault_path("workspaces/local-old/a/b.md")
                .as_deref(),
            Some("workspaces/local-new/a/b.md")
        );
        assert_eq!(
            a.current_vault_path("workspaces/local-old").as_deref(),
            Some("workspaces/local-new")
        );
        assert_eq!(a.current_vault_path("workspaces/local-new/a"), None);
        assert!(Aliases::default().same_vault_path("a/b", "a/b"));
    }

    #[test]
    fn workspace_aliases_without_the_table_are_empty() {
        let conn = Connection::open_in_memory().unwrap();
        let a = Aliases::load(&conn);
        assert!(a.is_empty());
        assert_eq!(a.ids_for("local-x"), vec!["local-x"]);
    }
}
