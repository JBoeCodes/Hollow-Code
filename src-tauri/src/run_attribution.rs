//! Run ledger v2 attribution (master plan F2, 2026-10-08): which account
//! paid for a run, which workspace and department it was for, and what kind
//! of work it was. Written onto the run's `usage_ledger` row
//! (`session_store::set_usage_attribution`) when the run's process launches
//! (`usage_ledger::UsageRun::launched`), so every launch point is covered
//! without a call-site change.
//!
//! - **Account.** [`account_ref`] is a hashed label of the Claude account id
//!   (`acct-<12 hex>`), or `login` for Claude Code's own login. Never the
//!   token, never the account's name: tokens stay in the Keychain
//!   (`claude_accounts`). Other agents record no account (unknown).
//! - **Workspace.** The workspace whose folder (or build worktree) the run's
//!   process starts in (`workspaces::workspace_for_cwd`). A run anywhere else
//!   records none.
//! - **Department.** The workspace brief's `department:` field, one of
//!   [`DEPARTMENTS`] (decision 3). Anything else is unknown, never guessed.
//! - **Work kind.** One of [`WORK_KINDS`], inferred: the brief's `work_kind:`
//!   field when set (the person's correction), else `code` for a build
//!   workspace, else `admin` for the Main agent, else unknown.
//!
//! Old rows have none of these and read as `unknown` (`ledger_query`).
//! Everything here is read-only and takes no lock: the ledger's lock order
//! (`usage_ledger` module doc) is unchanged.

use sha2::{Digest, Sha256};
use std::path::Path;

/// The departments a workspace can belong to (decision 3, 2026-10-08).
pub const DEPARTMENTS: [&str; 6] = ["Operations", "Finance", "HR", "IT", "Revenue", "AI"];

/// The kinds of work a run is attributed to.
pub const WORK_KINDS: [&str; 6] = ["research", "document", "code", "data", "admin", "meeting"];

/// What the ledger shows for a value nobody recorded.
pub const UNKNOWN: &str = "unknown";

/// The brief's frontmatter keys.
pub const DEPARTMENT_KEY: &str = "department";
pub const WORK_KIND_KEY: &str = "work_kind";

/// A department as stored, case-insensitively matched to [`DEPARTMENTS`].
pub fn normalize_department(raw: &str) -> Option<&'static str> {
    let raw = raw.trim().trim_matches(|c| c == '"' || c == '\'');
    DEPARTMENTS
        .iter()
        .copied()
        .find(|d| d.eq_ignore_ascii_case(raw))
}

/// A work kind as stored, case-insensitively matched to [`WORK_KINDS`].
pub fn normalize_work_kind(raw: &str) -> Option<&'static str> {
    let raw = raw.trim().trim_matches(|c| c == '"' || c == '\'');
    WORK_KINDS
        .iter()
        .copied()
        .find(|k| k.eq_ignore_ascii_case(raw))
}

/// The ledger's label for a Claude account id: `login` for Claude Code's own
/// login, else `acct-` and the first 12 hex digits of a SHA-256 of the id.
/// The id is not a secret, but the ledger never needs it: the label only has
/// to tell accounts apart and stay stable.
pub fn account_ref(account: &str) -> String {
    let account = account.trim();
    if account.is_empty() || account == crate::claude_accounts::LOGIN_ID {
        return crate::claude_accounts::LOGIN_ID.to_string();
    }
    let digest = Sha256::digest(format!("hollow-account-ref/v1/{account}").as_bytes());
    let hex: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("acct-{hex}")
}

/// A run's attribution, as written onto its ledger row. `None` is unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attribution {
    pub account_ref: Option<String>,
    pub workspace_id: Option<String>,
    pub department: Option<String>,
    pub work_kind: Option<String>,
}

impl Attribution {
    pub fn is_empty(&self) -> bool {
        *self == Attribution::default()
    }
}

/// The department and work kind a workspace's brief declares (absent or
/// invalid values are `None`).
pub fn declared(root: &Path, workspace_id: &str) -> (Option<&'static str>, Option<&'static str>) {
    let Ok(path) =
        crate::vault::safe_join(root, &format!("workspaces/{workspace_id}/WORKSPACE.md"))
    else {
        return (None, None);
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return (None, None);
    };
    let (fm, _) = crate::frontmatter::parse_map(&text);
    (
        fm.get(DEPARTMENT_KEY).and_then(|v| normalize_department(v)),
        fm.get(WORK_KIND_KEY).and_then(|v| normalize_work_kind(v)),
    )
}

/// The workspace part of a run's attribution, from the folder its process
/// starts in. Errors (an unreadable brief, a folder that no longer exists)
/// are unknown, never a failed run.
pub fn for_cwd(root: &Path, cwd: &Path) -> Attribution {
    let Ok(Some(workspace)) = crate::workspaces::workspace_for_cwd(root, cwd) else {
        return Attribution::default();
    };
    let (department, declared_kind) = declared(root, &workspace.id);
    let work_kind = declared_kind.or_else(|| infer_work_kind(&workspace));
    Attribution {
        account_ref: None,
        workspace_id: Some(workspace.id),
        department: department.map(str::to_string),
        work_kind: work_kind.map(str::to_string),
    }
}

/// The work kind of a workspace that declares none.
fn infer_work_kind(workspace: &crate::workspaces::Workspace) -> Option<&'static str> {
    if workspace.build.is_some() {
        Some("code")
    } else if workspace.id == crate::workspaces::MAIN_AGENT_ID {
        Some("admin")
    } else {
        None
    }
}

/// Set (or, with an empty value, clear) a workspace's `department:` and
/// `work_kind:` (`workspaces.set_department`). Refused for the built-in
/// workspaces and for tracker-managed ones.
/// Returns the workspace's attribution as it now reads. Runs already in the
/// ledger keep what they recorded.
pub fn set_workspace(
    root: &Path,
    id: &str,
    department: Option<&str>,
    work_kind: Option<&str>,
) -> Result<serde_json::Value, String> {
    use crate::frontmatter::KeyEdit;
    crate::workspaces::summary(root, id)?;
    if crate::workspaces::is_quick_chats(id) || crate::workspaces::is_main_agent(id) {
        return Err("Chats and the COO are built in: their department can't be set.".into());
    }
    if crate::workspaces::tracker_managed(root, id) {
        return Err(
            "This workspace follows its task tracker project; its department can't be set here."
                .into(),
        );
    }
    let mut edits: Vec<(&str, KeyEdit)> = Vec::new();
    if let Some(raw) = department {
        if raw.trim().is_empty() {
            edits.push((DEPARTMENT_KEY, KeyEdit::Remove));
        } else {
            let d = normalize_department(raw).ok_or_else(|| {
                format!(
                    "Unknown department '{raw}'. Use one of: {}.",
                    DEPARTMENTS.join(", ")
                )
            })?;
            edits.push((DEPARTMENT_KEY, KeyEdit::Set(d.into())));
        }
    }
    if let Some(raw) = work_kind {
        if raw.trim().is_empty() {
            edits.push((WORK_KIND_KEY, KeyEdit::Remove));
        } else {
            let k = normalize_work_kind(raw).ok_or_else(|| {
                format!(
                    "Unknown work kind '{raw}'. Use one of: {}.",
                    WORK_KINDS.join(", ")
                )
            })?;
            edits.push((WORK_KIND_KEY, KeyEdit::Set(k.into())));
        }
    }
    if edits.is_empty() {
        return Err("Give a department and/or a work kind.".into());
    }
    // Compare-and-swap; the mtime is kept unless a chat runs there
    // (concurrency PR 2, 2026-10-20).
    crate::shared_docs::update_doc(root, id, "WORKSPACE.md", |text| {
        crate::frontmatter::patch(text, &edits, None)
    })?;
    let (department, work_kind) = declared(root, id);
    Ok(serde_json::json!({
        "id": id,
        "department": department.unwrap_or(UNKNOWN),
        "workKind": work_kind.unwrap_or(UNKNOWN),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(root: &Path, id: &str, extra: &str) {
        let dir = root.join("workspaces").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("WORKSPACE.md"),
            format!(
                "---\nhollow_workspace: 1\ntitle: \"{id}\"\nstatus: active\n{extra}---\n\n# {id}\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn account_ref_is_a_stable_label_never_the_id() {
        let a = account_ref("tech-7f3a");
        assert_eq!(a, account_ref("tech-7f3a"), "stable");
        assert!(a.starts_with("acct-") && a.len() == 17, "{a}");
        assert!(!a.contains("tech"), "the id never appears: {a}");
        assert_ne!(a, account_ref("admin-1b2c"));
        assert_eq!(account_ref("login"), "login");
        assert_eq!(account_ref(""), "login");
    }

    #[test]
    fn departments_and_work_kinds_match_the_fixed_lists_only() {
        assert_eq!(normalize_department("finance"), Some("Finance"));
        assert_eq!(normalize_department(" \"HR\" "), Some("HR"));
        assert_eq!(normalize_department("Marketing"), None);
        assert_eq!(normalize_work_kind("Code"), Some("code"));
        assert_eq!(normalize_work_kind("coding"), None);
    }

    #[test]
    fn a_run_in_a_workspace_takes_its_declared_department_and_kind() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        workspace(root, "payroll", "department: finance\nwork_kind: data\n");
        workspace(root, "notes", "department: Marketing\n");
        let a = for_cwd(root, &root.join("workspaces/payroll"));
        assert_eq!(a.workspace_id.as_deref(), Some("payroll"));
        assert_eq!(a.department.as_deref(), Some("Finance"));
        assert_eq!(a.work_kind.as_deref(), Some("data"));
        // An unlisted department is unknown, not guessed.
        let b = for_cwd(root, &root.join("workspaces/notes"));
        assert_eq!(b.workspace_id.as_deref(), Some("notes"));
        assert_eq!((b.department, b.work_kind), (None, None));
        // Outside every workspace: nothing.
        assert!(for_cwd(root, root).is_empty());
    }

    #[test]
    fn set_workspace_writes_only_listed_values_and_can_clear_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        workspace(root, "ops", "");
        let out = set_workspace(root, "ops", Some("operations"), Some("Admin")).unwrap();
        assert_eq!(out["department"], "Operations");
        assert_eq!(out["workKind"], "admin");
        let text = std::fs::read_to_string(root.join("workspaces/ops/WORKSPACE.md")).unwrap();
        assert!(
            text.contains("department: Operations") && text.contains("title: \"ops\""),
            "{text}"
        );
        assert!(set_workspace(root, "ops", Some("Marketing"), None).is_err());
        assert!(set_workspace(root, "ops", None, Some("coding")).is_err());
        assert!(set_workspace(root, "ops", None, None).is_err());
        let out = set_workspace(root, "ops", Some(""), None).unwrap();
        assert_eq!(
            (&out["department"], &out["workKind"]),
            (&serde_json::json!(UNKNOWN), &serde_json::json!("admin"))
        );
        // A legacy Notion workspace is a plain one (2026-11-10).
        workspace(root, "notion-abc00000000000000000000000000000", "");
        assert!(set_workspace(
            root,
            "notion-abc00000000000000000000000000000",
            Some("HR"),
            None
        )
        .is_ok());
        assert!(set_workspace(root, "missing", Some("HR"), None).is_err());
    }
}
