//! Config bootstrap helpers and the About-you profile starters.
//!
//! The Hollow folder itself is prepared by `directory_connections`
//! (Settings, Sources, Hollow folder and first-run onboarding). What stays here:
//! - the atomic, merge-preserving config writer every config change uses;
//! - the retired workspace-source switch (always `obsidian`, kept so old
//!   configs normalize on save);
//! - Settings, Personalization, Your profile, Create profile notes: the five `me/` starters,
//!   strictly additive (an existing note is never overwritten).
//!
//! The old scaffold wizard (folder checklist, welcome note, sample task and
//! project, routine workflows and the `.automation-seeded` routine seed) was
//! removed on 2026-09-26: nothing called it, and it created folders the app
//! no longer reads. See docs/BUILD_LOG.md.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

// Wire types (serde camelCase, mirrored in src/types.ts)

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    /// False when no config.json could be loaded at boot — the frontend
    /// renders the Onboarding wizard instead of the app shell.
    pub configured: bool,
    pub config_path: Option<String>,
    pub config_error: Option<String>,
    pub vault_path: Option<String>,
    pub vault_exists: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSourceUpdate {
    /// Only `obsidian`. Older callers' Notion fields (`twoWaySync`,
    /// `notionToken`, data source ids) are ignored: Notion sync is gone
    /// (2026-11-10) and a stored `notion` block is left as it is.
    pub source: String,
}

pub fn expand_tilde(input: &str) -> PathBuf {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    } else if trimmed == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(trimmed)
}

/// Where a freshly-written config.json should live: `$HOLLOW_CONFIG` when the
/// user pinned one, else the platform config dir `config::locate_config`
/// checks first.
pub fn config_write_path() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("HOLLOW_CONFIG") {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let home = dirs::home_dir().ok_or_else(|| "home dir not found".to_string())?;
    // A candidate build writes its own (instance.rs), never the everyday app's.
    let name = crate::instance::current().support_name();
    #[cfg(target_os = "macos")]
    let dir = home.join("Library").join("Application Support").join(name);
    #[cfg(not(target_os = "macos"))]
    let dir = home.join(".config").join(name);
    Ok(dir.join("config.json"))
}

/// A missing config is a first-run case; unreadable or invalid existing
/// configuration must never be mistaken for an empty map and overwritten.
pub(crate) fn load_config_map(path: &Path) -> Result<BTreeMap<String, serde_json::Value>, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| format!("Existing config is invalid and was preserved: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(format!(
            "Existing config could not be read and was preserved: {error}"
        )),
    }
}

/// Publish config through a private, exclusively created sibling temporary
/// file. Preserve an existing config's permissions and symlink target; newly
/// created Unix configs begin at 0600 because they may contain credentials.
pub(crate) fn publish_config(
    path: &Path,
    map: &BTreeMap<String, serde_json::Value>,
) -> Result<(), String> {
    publish_config_mode(path, map, false)
}

/// `publish_config` for a file that holds secrets: always owner-only (0600),
/// whatever mode the file had before.
pub(crate) fn publish_private_config(
    path: &Path,
    map: &BTreeMap<String, serde_json::Value>,
) -> Result<(), String> {
    publish_config_mode(path, map, true)
}

fn publish_config_mode(
    path: &Path,
    map: &BTreeMap<String, serde_json::Value>,
    private: bool,
) -> Result<(), String> {
    let text =
        serde_json::to_string_pretty(map).map_err(|error| format!("serialize config: {error}"))?;
    publish_bytes_mode(path, (text + "\n").as_bytes(), private)
}

/// Publish exact bytes the same way (an owner switch's rollback puts back
/// config.json byte for byte, `sources::migration`).
pub(crate) fn publish_config_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    publish_bytes_mode(path, bytes, false)
}

fn publish_bytes_mode(path: &Path, bytes: &[u8], private: bool) -> Result<(), String> {
    let target = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => path
            .canonicalize()
            .map_err(|error| format!("resolve config symlink without replacing it: {error}"))?,
        Ok(_) => path.to_path_buf(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(format!("inspect config: {error}")),
    };
    let permissions = match fs::metadata(&target) {
        // A private file keeps the temporary's 0600 instead.
        Ok(_) if private => None,
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("inspect config permissions: {error}")),
    };
    let parent = target.parent().ok_or("config has no parent directory")?;
    fs::create_dir_all(parent).map_err(|error| format!("create config directory: {error}"))?;
    let temporary = parent.join(format!(
        ".hollow-config-{}-{:016x}.tmp",
        std::process::id(),
        rand::random::<u64>()
    ));
    publish_config_at(&target, &temporary, bytes, permissions)
}

fn publish_config_at(
    target: &Path,
    temporary: &Path,
    bytes: &[u8],
    permissions: Option<fs::Permissions>,
) -> Result<(), String> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Open before establishing cleanup ownership: a pre-existing temporary
    // (including a symlink) must neither be followed nor removed on failure.
    let mut file = options
        .open(temporary)
        .map_err(|error| format!("create private config temporary: {error}"))?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|error| format!("write config: {error}"))?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)
                .map_err(|error| format!("preserve config permissions: {error}"))?;
        }
        file.sync_all()
            .map_err(|error| format!("sync config: {error}"))?;
        fs::rename(temporary, target).map_err(|error| format!("publish config: {error}"))
    })();
    let _ = fs::remove_file(temporary);
    result
}

pub fn set_config_workspace_source(
    update: &WorkspaceSourceUpdate,
    path: &Path,
) -> Result<(), String> {
    let mut map = load_config_map(path)?;
    validate_workspace_source(&update.source)?;
    set_workspace_source_values(&mut map);
    publish_config(path, &map)
}

/// Writes the source keys only; a stored `notion` block (the importer's
/// token and data source ids) is never touched.
fn set_workspace_source_values(map: &mut BTreeMap<String, serde_json::Value>) {
    let source = crate::config::WorkspaceSource::Obsidian;
    map.insert(
        "workspaceSource".into(),
        serde_json::Value::String(source.as_str().into()),
    );
    let domains = map
        .entry("sourceOfTruth".into())
        .or_insert_with(|| serde_json::json!({}));
    if !domains.is_object() {
        *domains = serde_json::json!({});
    }
    let domains = domains.as_object_mut().expect("object assigned above");
    domains.insert("tasks".into(), serde_json::json!("vault"));
    domains.insert("projects".into(), serde_json::json!("vault"));
}

fn validate_workspace_source(source: &str) -> Result<(), String> {
    if source.trim().eq_ignore_ascii_case("obsidian") {
        Ok(())
    } else {
        Err("Hollow uses local files for workspaces and Library. The only configuration value is obsidian.".into())
    }
}

pub fn default_claude_projects_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".claude")
        .join("projects")
}

// ---------------------------------------------------------------------------
// About you starters
// ---------------------------------------------------------------------------

/// The five operator-profile notes, in the order Settings lists them.
pub const PROFILE_NOTES: &[(&str, &str)] = &[
    ("me/Identity.md", ME_IDENTITY),
    ("me/Communication.md", ME_COMMUNICATION),
    ("me/Anti-patterns.md", ME_ANTIPATTERNS),
    ("me/Guardrails.md", ME_GUARDRAILS),
    ("me/Decisions.md", ME_DECISIONS),
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileNotesResult {
    pub created: u32,
    pub skipped: u32,
    pub errors: Vec<String>,
}

/// Create any missing `me/` profile note from its starter. Existing notes
/// always win. Attributed as Hollow's own write so a run active at the time
/// is not flagged for changing `me/`.
pub fn create_profile_notes(vault: &Path) -> Result<ProfileNotesResult, String> {
    if !vault.is_dir() {
        return Err(format!("vault folder not found: {}", vault.display()));
    }
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut out = ProfileNotesResult {
        created: 0,
        skipped: 0,
        errors: Vec::new(),
    };
    for (rel, template) in PROFILE_NOTES {
        let target = vault.join(rel);
        if target.exists() {
            out.skipped += 1;
            continue;
        }
        if let Some(parent) = target.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                out.errors.push(format!("{rel}: {e}"));
                continue;
            }
        }
        let content = template.replace("{{today}}", &today);
        match crate::vault::create_note(vault, rel, &content) {
            Ok(_) => {
                out.created += 1;
                crate::run_guard::note_host_vault_write(vault, rel, None, Some(content.as_bytes()));
            }
            Err(e) => out.errors.push(format!("{rel}: {e}")),
        }
    }
    Ok(out)
}

const ME_IDENTITY: &str = r#"---
title: Identity
last_updated: {{today}}
last_reviewed:
review_cadence: 90
---

# Identity

Who I am. Older unscoped conversations and automations can use this profile.
Attach it explicitly to a workspace when relevant. Keep it short and current.

- **Name:** (fill in)
- **Role:** (fill in)
- **Company:** (fill in)

## What I'm working on

- (your current focus areas, one per line)

## Context that always helps

- (timezone, working hours, team, tools you live in)
"#;

const ME_COMMUNICATION: &str = r#"---
title: Communication
last_updated: {{today}}
last_reviewed:
review_cadence: 90
---

# Communication

How I want agents to communicate with me. Be opinionated — this is read at
the start of older unscoped sessions. Attach it explicitly to workspaces when needed.

- Preferred tone: (direct? formal? casual?)
- Length: (short answers vs. full context)
- Formatting: (bullets vs. prose)

## Always

- (e.g. lead with the answer, then the reasoning)

## Never

- (e.g. don't pad replies with caveats I didn't ask for)
"#;

const ME_ANTIPATTERNS: &str = r#"---
title: Anti-patterns
last_updated: {{today}}
last_reviewed:
review_cadence: 60
---

# Anti-patterns

Specific behaviors to stop. When something annoys you twice, write it here
once — every future session starts knowing it.

- (e.g. suggesting a rewrite when I asked for a one-line fix)
- (e.g. summarizing a file back to me instead of answering the question)
"#;

const ME_GUARDRAILS: &str = r#"---
title: Guardrails
last_updated: {{today}}
last_reviewed:
review_cadence: 90
---

# Guardrails

Hard rules that always apply, regardless of what a task seems to call for.

- Hollow has no built-in email, Slack or Teams features. Use local notes, tasks, projects, Calendar and recordings, and only connectors the user shared in Hollow.
- Never delete or move files outside this vault.
- When unsure which of two people/projects I mean, ask — don't guess.
"#;

const ME_DECISIONS: &str = r#"---
title: Decisions
last_updated: {{today}}
last_reviewed:
review_cadence: 30
---

# Decisions

Settled decisions, newest first. Once something lands here, the agent treats it
as decided — no re-litigating old debates in new sessions.

## {{today}} — Started using Hollow

- The vault is the single source of truth; tools sync into it, not around it.
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn profile_notes_are_created_once_and_never_overwritten() {
        let tmp = TempDir::new().unwrap();
        let v = tmp.path();
        fs::create_dir_all(v.join("me")).unwrap();
        fs::write(v.join("me/Identity.md"), "mine").unwrap();
        let r = create_profile_notes(v).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!((r.created, r.skipped), (4, 1));
        assert_eq!(
            fs::read_to_string(v.join("me/Identity.md")).unwrap(),
            "mine"
        );
        let comm = fs::read_to_string(v.join("me/Communication.md")).unwrap();
        assert!(comm.contains("review_cadence: 90"));
        assert!(!comm.contains("{{"));
        let again = create_profile_notes(v).unwrap();
        assert_eq!((again.created, again.skipped), (0, 5));
    }

    #[test]
    fn profile_notes_during_an_automated_run_are_not_the_runs_change() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path();
        let run = crate::run_guard::begin_for_test(vault, "chat", &[]);
        let r = create_profile_notes(vault).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(vault.join("me/Guardrails.md").is_file());
        assert!(crate::run_guard::finish_for_test(vault, &run).is_empty());
    }

    #[test]
    fn profile_notes_refuse_a_missing_vault() {
        let tmp = TempDir::new().unwrap();
        assert!(create_profile_notes(&tmp.path().join("nope")).is_err());
    }

    #[test]
    fn workspace_source_update_validates_and_preserves_notion_config() {
        let tmp = TempDir::new().unwrap();
        let cfg_path = tmp.path().join("config.json");
        fs::write(
            &cfg_path,
            r#"{ "vaultPath": "/vault", "notionToken": "legacy-secret", "custom": 7,
                "notion": { "token": "kept", "twoWaySync": true, "taskDataSourceId": "tasks-id" } }"#,
        )
        .unwrap();
        let missing = WorkspaceSourceUpdate {
            source: "notion".into(),
        };
        assert!(set_config_workspace_source(&missing, &cfg_path).is_err());

        // An older caller's Notion fields are ignored, not written.
        let configured: WorkspaceSourceUpdate = serde_json::from_value(serde_json::json!({
            "source": "obsidian", "twoWaySync": false, "notionToken": "",
            "taskDataSourceId": "other", "projectDataSourceId": "projects-id"
        }))
        .unwrap();
        set_config_workspace_source(&configured, &cfg_path).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&cfg_path).unwrap()).unwrap();
        assert_eq!(parsed["workspaceSource"], "obsidian");
        assert_eq!(parsed["sourceOfTruth"]["tasks"], "vault");
        assert_eq!(parsed["sourceOfTruth"]["projects"], "vault");
        // The stored Notion block stays as it was: the importer reads it.
        assert_eq!(parsed["notion"]["token"], "kept");
        assert_eq!(parsed["notion"]["twoWaySync"], true);
        assert_eq!(parsed["notion"]["taskDataSourceId"], "tasks-id");
        assert!(parsed["notion"].get("projectDataSourceId").is_none());
        assert_eq!(parsed["notionToken"], "legacy-secret");
        assert_eq!(parsed["custom"], 7);
    }

    #[test]
    fn invalid_existing_config_is_preserved_by_workspace_and_vault_updates() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.json");
        let original = "{partial config with credentials";
        fs::write(&path, original).unwrap();
        assert!(set_config_workspace_source(
            &WorkspaceSourceUpdate {
                source: "obsidian".into(),
            },
            &path
        )
        .unwrap_err()
        .contains("was preserved"));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn changing_authority_preserves_unrelated_domain_configuration() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.json");
        fs::write(&path, r#"{"sourceOfTruth":{"tasks":"notion","otherDomain":{"custom":true}},"notion":{"taskProperties":{"title":"Custom title"}}}"#).unwrap();
        set_config_workspace_source(
            &WorkspaceSourceUpdate {
                source: "obsidian".into(),
            },
            &path,
        )
        .unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["sourceOfTruth"]["otherDomain"]["custom"], true);
        assert_eq!(value["sourceOfTruth"]["tasks"], "vault");
        assert_eq!(value["notion"]["taskProperties"]["title"], "Custom title");
    }

    #[cfg(unix)]
    #[test]
    fn config_atomic_save_preserves_private_permissions_and_defaults_to_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("config.json");
        let map = BTreeMap::from([("notionToken".into(), serde_json::json!("private-token"))]);
        publish_config(&path, &map).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for mode in [0o600, 0o640] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            publish_config(&path, &map).unwrap();
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                mode
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn config_save_preserves_symlink_and_rejects_symlink_temporary() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("actual-config.json");
        let link = tmp.path().join("config.json");
        fs::write(&target, "{}").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&target, &link).unwrap();
        let mut map = load_config_map(&link).unwrap();
        map.insert("vaultPath".into(), serde_json::json!("/new-vault"));
        publish_config(&link, &map).unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(fs::read_to_string(&target).unwrap().contains("/new-vault"));
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let sentinel = tmp.path().join("sentinel.txt");
        let temporary = tmp.path().join("occupied.tmp");
        fs::write(&sentinel, "Do not touch").unwrap();
        symlink(&sentinel, &temporary).unwrap();
        assert!(publish_config_at(&target, &temporary, b"replacement", None).is_err());
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "Do not touch");
        assert!(fs::symlink_metadata(&temporary)
            .unwrap()
            .file_type()
            .is_symlink());
    }
}
