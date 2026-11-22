//! Settings export and import (overnight F7): the person's own copy of
//! Hollow's settings, to move them to another Mac or back up a setup.
//!
//! **What travels.** Every key in [`crate::settings_keys::KEYS`] (the same
//! registry agents' `get_settings` reads) except secrets: a `Type::Secret`
//! key is never written to the file and never read from one. Values are read
//! and written through each key's own owner, the backend getter/setter or the
//! webview store, exactly as `settings.change` does.
//!
//! **Import is two steps.** [`plan`] (pure) compares a file with the current
//! values: the changes (old → new, as Settings shows them), how many keys
//! already match, and what is ignored and why (unknown key, secret, a value
//! of the wrong type). Its `hash` binds the apply to what the preview showed:
//! if the file or a setting changed in between, nothing is applied.
//!
//! **Not carried:** [`PERSONAL`] keys (palette history, folded workspace
//! ids) mean nothing on another Mac; `shortcuts` travels as its settings
//! only (`enabled`, `keys`), never its computed warnings. A value the file
//! leaves at its default (null) is not a reset: Hollow can't put a key back
//! to its default from a file, so the preview says so.
//!
//! **Who.** Three Tauri commands (the file pickers are the window's part) and
//! the COO's three actions (`settings.settings_export`,
//! `settings_import_preview`, `settings_import_apply`), which give a path in
//! words and share the same inner functions, run as the COO's own caller. A
//! COO export never overwrites and never writes inside the vault; its
//! preview and its apply card show values redacted ([`for_agent`]: a nested
//! `apiKey` too), and an apply is a confirm card listing every change, bound
//! to the preview's exact hash.

use crate::app_actions::{self, Caller, Cx};
use crate::app_mcp::AppAccess;
use crate::settings_keys::{self as keys, Owner, Type};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

/// The file's format version.
pub(crate) const FORMAT: u64 = 1;
/// The largest settings file Hollow reads.
const MAX_FILE: u64 = 1024 * 1024;

/// One setting the import would change.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub key: String,
    pub summary: String,
    pub old: String,
    pub new: String,
    /// A confirm-tier key: what agents may do, what runs on its own or
    /// spends, accounts. The preview lists these first.
    pub asks: bool,
    /// The value written, as the file has it.
    #[serde(skip)]
    pub value: Value,
}

/// A key in the file the import leaves alone, and why.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Ignored {
    pub key: String,
    pub why: String,
}

/// What importing a file would do.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub changes: Vec<Change>,
    pub unchanged: usize,
    pub ignored: Vec<Ignored>,
    /// When the file was exported, as it says.
    pub exported_at: Option<String>,
    /// Binds the apply to this preview.
    pub hash: String,
}

/// What an import changed.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub applied: usize,
    pub failed: Vec<Ignored>,
}

/// What an export wrote.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    pub path: String,
    pub count: usize,
    /// Keys that couldn't be read (left out).
    pub skipped: Vec<String>,
}

/// Keys that are personal history rather than settings, with why.
pub(crate) const PERSONAL: &[(&str, &str)] = &[
    (
        "paletteRecents",
        "the palette's recent items: personal history, not a setting",
    ),
    (
        "railCollapsed",
        "folded workspace ids from this Mac: personal, not a setting",
    ),
];

/// Whether a key may travel in a file: everything but secrets and
/// [`PERSONAL`] keys.
pub(crate) fn exportable(key: &keys::Key) -> bool {
    !matches!(key.kind, Type::Secret) && !PERSONAL.iter().any(|(k, _)| *k == key.key)
}

/// A value as it travels: `shortcuts` without its computed `warnings`.
pub(crate) fn portable(key: &str, value: Value) -> Value {
    match (key, value) {
        ("shortcuts", Value::Object(mut map)) => {
            map.retain(|field, _| field == "enabled" || field == "keys");
            Value::Object(map)
        }
        (_, value) => value,
    }
}

/// The context reads and writes run in: the COO's own `caller` for its
/// actions (L13 review: an agent's import stays the agent's, through the
/// same non-command setters), else the person's (no run, no card).
fn cx_as(app: &tauri::AppHandle, caller: Option<Caller>) -> Cx {
    Cx {
        host: Box::new(app.clone()),
        caller: caller.unwrap_or_else(|| Caller {
            run_id: String::new(),
            surface: "settings".into(),
            scope: "settings-transfer".into(),
            access: AppAccess::Write,
            depth: 0,
            perm: "default".into(),
        }),
        approval: None,
    }
}

/// What an agent is shown in place of a secret.
const MASK: &str = "••••";

/// Whether a field name looks like it holds a secret (`apiKey`,
/// `access_token`, `password`, ...), however it is cased or separated.
pub(crate) fn secret_name(name: &str) -> bool {
    let name: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    [
        "apikey",
        "token",
        "secret",
        "password",
        "passwd",
        "passphrase",
        "credential",
        "privatekey",
        "authorization",
        "bearer",
        "cookie",
    ]
    .iter()
    .any(|word| name.contains(word))
}

/// `value` as an agent may see it (L13 review): every field whose name
/// looks secret masked at any depth (`heartbeat.config.apiKey`), and every
/// string through [`crate::claude_runner::redact`]. Hashing never uses this.
pub(crate) fn redacted_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(field, v)| {
                    let blank = v.is_null() || v.as_str().is_some_and(str::is_empty);
                    let shown = if secret_name(field) && !blank {
                        json!(MASK)
                    } else {
                        redacted_value(v)
                    };
                    (field.clone(), shown)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redacted_value).collect()),
        Value::String(text) => Value::String(crate::claude_runner::redact(text)),
        other => other.clone(),
    }
}

/// A setting's value as an agent-facing preview or card shows it: a
/// `Secret` key or a key named like a secret masked whole, else
/// [`keys::shown`] of [`redacted_value`].
pub(crate) fn shown_for_agent(key: &keys::Key, value: &Value) -> String {
    if !value.is_null() && (matches!(key.kind, Type::Secret) || secret_name(key.key)) {
        return MASK.into();
    }
    keys::shown(key, &redacted_value(value))
}

/// `preview` as an agent (or the card an agent's apply asks on) may see it:
/// each change's old and new through [`shown_for_agent`] (`current` holds
/// the old values), and every reason and date through
/// [`crate::claude_runner::redact`]. The hash and the values applied stay
/// exact. Pure.
pub(crate) fn for_agent(mut preview: Preview, current: &Map<String, Value>) -> Preview {
    for change in &mut preview.changes {
        let Some(key) = keys::find(&change.key) else {
            change.old = MASK.into();
            change.new = MASK.into();
            continue;
        };
        change.old = shown_for_agent(key, current.get(&change.key).unwrap_or(&Value::Null));
        change.new = shown_for_agent(key, &change.value);
    }
    for ignored in &mut preview.ignored {
        ignored.key = crate::claude_runner::redact(&ignored.key);
        ignored.why = crate::claude_runner::redact(&ignored.why);
    }
    preview.exported_at = preview
        .exported_at
        .as_deref()
        .map(crate::claude_runner::redact);
    preview
}

/// A settings file inside the vault would be indexed and synced, and an
/// agent's write there bypasses History (L13 review): refused, with the
/// vault and the target's folder compared canonically.
pub(crate) fn outside_vault(vault: &Path, target: &Path) -> Result<(), String> {
    let real_vault = vault.canonicalize().unwrap_or_else(|_| vault.to_path_buf());
    let real_parent = target
        .parent()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
    if target.starts_with(vault)
        || target.starts_with(&real_vault)
        || real_parent.is_some_and(|p| p.starts_with(&real_vault))
    {
        return Err("Save the settings file outside the vault (a file there would be indexed and synced). Name a path in another folder; nothing was written.".into());
    }
    Ok(())
}

/// Every exportable key's current value; keys that can't be read are named
/// in the second list instead of failing the whole read.
async fn current(cx: &Cx) -> (Map<String, Value>, Vec<String>) {
    let ui_keys: Vec<&str> = keys::KEYS
        .iter()
        .filter(|k| exportable(k) && matches!(k.owner, Owner::Ui))
        .map(|k| k.key)
        .collect();
    let ui = if ui_keys.is_empty() {
        Ok(json!({}))
    } else {
        app_actions::ui_bridge(cx, "get", json!({ "keys": ui_keys }))
    };
    let mut values = Map::new();
    let mut skipped = Vec::new();
    for key in keys::KEYS.iter().filter(|k| exportable(k)) {
        let value = match key.owner {
            Owner::Backend { get, .. } => get(cx.fresh()).await,
            Owner::Ui => ui
                .as_ref()
                .map(|v| v[key.key].clone())
                .map_err(Clone::clone),
        };
        match value {
            Ok(value) => {
                values.insert(key.key.to_string(), portable(key.key, value));
            }
            Err(_) => skipped.push(key.key.to_string()),
        }
    }
    (values, skipped)
}

/// The file an export writes.
pub(crate) fn file_body(values: &Map<String, Value>, exported_at: &str) -> Value {
    json!({
        "hollowSettings": FORMAT,
        "exportedAt": exported_at,
        "appVersion": env!("CARGO_PKG_VERSION"),
        "settings": values,
    })
}

/// What importing `file` over `current` would change. `unreadable` are keys
/// whose current value couldn't be read: they are left alone, never shown as
/// a change from nothing. Pure.
pub(crate) fn plan(
    file: &Value,
    current: &Map<String, Value>,
    unreadable: &[String],
) -> Result<Preview, String> {
    if file.get("hollowSettings").and_then(Value::as_u64) != Some(FORMAT) {
        return Err(
            "That isn't a Hollow settings file (it has no \"hollowSettings\": 1). Nothing changed."
                .into(),
        );
    }
    let settings = file
        .get("settings")
        .and_then(Value::as_object)
        .ok_or("The settings file has no \"settings\" object. Nothing changed.")?;
    let mut changes = Vec::new();
    let mut ignored = Vec::new();
    let mut unchanged = 0;
    for (name, value) in settings {
        let Some(key) = keys::find(name) else {
            ignored.push(Ignored {
                key: name.clone(),
                why: "not a setting this version of Hollow has".into(),
            });
            continue;
        };
        if matches!(key.kind, Type::Secret) {
            ignored.push(Ignored {
                key: name.clone(),
                why: "a secret: never imported from a file".into(),
            });
            continue;
        }
        if let Some((_, why)) = PERSONAL.iter().find(|(k, _)| *k == key.key) {
            ignored.push(Ignored {
                key: name.clone(),
                why: (*why).into(),
            });
            continue;
        }
        if unreadable.iter().any(|k| k == name) {
            ignored.push(Ignored {
                key: name.clone(),
                why: "Hollow couldn't read its current value, so it can't show the change".into(),
            });
            continue;
        }
        let value = &portable(name, value.clone());
        let old = current.get(name).cloned().unwrap_or(Value::Null);
        if value.is_null() && keys::validate(key, value).is_err() {
            // The file has this at its default; a file can't reset a key.
            if old.is_null() {
                unchanged += 1;
            } else {
                ignored.push(Ignored {
                    key: name.clone(),
                    why: "the file has it at its default; reset it in Settings if you want that"
                        .into(),
                });
            }
            continue;
        }
        if let Some(why) = key.refuse.and_then(|refuse| refuse(value)) {
            let _ = why;
            ignored.push(Ignored {
                key: name.clone(),
                why: "turning this on is done in Settings itself, not from a file".into(),
            });
            continue;
        }
        if let Err(error) = keys::validate(key, value) {
            ignored.push(Ignored {
                key: name.clone(),
                why: error,
            });
            continue;
        }
        if old == *value {
            unchanged += 1;
            continue;
        }
        changes.push(Change {
            key: name.clone(),
            summary: key.summary.to_string(),
            old: keys::shown(key, &old),
            new: keys::shown(key, value),
            asks: key.tier == crate::app_actions::Tier::Confirm,
            value: value.clone(),
        });
    }
    changes.sort_by(|a, b| b.asks.cmp(&a.asks).then_with(|| a.key.cmp(&b.key)));
    ignored.sort_by(|a, b| a.key.cmp(&b.key));
    let mut hasher = Sha256::new();
    for change in &changes {
        hasher.update(change.key.as_bytes());
        hasher.update([0]);
        hasher.update(change.value.to_string().as_bytes());
        hasher.update([0]);
        hasher.update(
            current
                .get(&change.key)
                .unwrap_or(&Value::Null)
                .to_string()
                .as_bytes(),
        );
        hasher.update([0xff]);
    }
    let hash: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Preview {
        changes,
        unchanged,
        ignored,
        exported_at: file
            .get("exportedAt")
            .and_then(Value::as_str)
            .map(str::to_string),
        hash,
    })
}

/// The file at `path`, parsed: absolute, at most 1 MB, JSON.
fn read_file(path: &str) -> Result<Value, String> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err("Pick the settings file from the file picker. Nothing changed.".into());
    }
    let meta =
        std::fs::metadata(path).map_err(|e| format!("Couldn't open the settings file: {e}"))?;
    if !meta.is_file() {
        return Err("That isn't a file. Pick the settings file. Nothing changed.".into());
    }
    let mut text = String::new();
    use std::io::Read;
    std::fs::File::open(path)
        .map_err(|e| format!("Couldn't open the settings file: {e}"))?
        .take(MAX_FILE + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("Couldn't read the settings file: {e}"))?;
    if text.len() as u64 > MAX_FILE {
        return Err("That file is too large to be a Hollow settings file. Nothing changed.".into());
    }
    serde_json::from_str(&text).map_err(|e| format!("The settings file isn't valid JSON: {e}"))
}

/// Run `work` the way the broker runs actions: on a blocking thread, with
/// the settings getters' (non-`Send`) futures driven by `block_on`.
async fn on_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

/// Export every non-secret setting to `path` (a `.json` file the person
/// picked in the save dialog).
#[tauri::command]
pub async fn settings_export(path: String, app: tauri::AppHandle) -> Result<Exported, String> {
    export_to(path, app, None).await
}

/// The body of `settings_export`. The person's (`agent` `None`) may replace
/// a file: that is the save dialog's own replace prompt. The COO's (its
/// caller) never replaces a file and never writes inside the vault, and
/// reads the settings as itself.
pub(crate) async fn export_to(
    path: String,
    app: tauri::AppHandle,
    agent: Option<Caller>,
) -> Result<Exported, String> {
    let target = std::path::PathBuf::from(&path);
    let json = target
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    if !target.is_absolute() || !json {
        return Err("Pick where to save the settings with the file picker (a .json file).".into());
    }
    let overwrite = agent.is_none();
    if !overwrite {
        let vault = tauri::Manager::state::<crate::commands::AppState>(&app)
            .config
            .vault_path
            .clone();
        outside_vault(&vault, &target)?;
    }
    on_blocking(move || {
        let cx = cx_as(&app, agent);
        let (values, skipped) = tauri::async_runtime::block_on(current(&cx));
        let body = file_body(&values, &chrono::Utc::now().to_rfc3339());
        let text = serde_json::to_string_pretty(&body).map_err(|e| e.to_string())?;
        // A fresh temp name beside the target, created new (never through
        // a link left at that name), then renamed over it.
        let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
        let tmp = target.with_file_name(format!(
            ".hollow-settings-{}-{stamp}.partial",
            std::process::id()
        ));
        {
            use std::io::Write;
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .map_err(|e| format!("Couldn't write the settings file: {e}"))?;
            if let Err(e) = out.write_all(format!("{text}\n").as_bytes()) {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("Couldn't write the settings file: {e}"));
            }
        }
        if overwrite {
            std::fs::rename(&tmp, &target).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("Couldn't write the settings file: {e}")
            })?;
        } else {
            // A hard link fails when the name exists: nothing is replaced.
            let linked = std::fs::hard_link(&tmp, &target);
            let _ = std::fs::remove_file(&tmp);
            linked.map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    "A file is already there. Choose a new file name: the COO doesn't replace files. Nothing was written.".to_string()
                } else {
                    format!("Couldn't write the settings file: {e}")
                }
            })?;
        }
        Ok(Exported {
            path,
            count: values.len(),
            skipped,
        })
    })
    .await
}

/// What importing the file at `path` would change. Reads only.
#[tauri::command]
pub async fn settings_import_preview(
    path: String,
    app: tauri::AppHandle,
) -> Result<Preview, String> {
    import_preview(path, app, None).await
}

/// The body of `settings_import_preview`, also the COO's: with `agent` (its
/// caller) the preview is read as the COO and comes back redacted
/// ([`for_agent`]); its hash is the same exact one.
pub(crate) async fn import_preview(
    path: String,
    app: tauri::AppHandle,
    agent: Option<Caller>,
) -> Result<Preview, String> {
    on_blocking(move || {
        let file = read_file(&path)?;
        let redact = agent.is_some();
        let (values, unreadable) = tauri::async_runtime::block_on(current(&cx_as(&app, agent)));
        let preview = plan(&file, &values, &unreadable)?;
        Ok(if redact {
            for_agent(preview, &values)
        } else {
            preview
        })
    })
    .await
}

/// Apply the import the person confirmed: only when the plan still hashes to
/// `expected_hash`; each change goes through its key's own setter.
#[tauri::command]
pub async fn settings_import_apply(
    path: String,
    expected_hash: String,
    app: tauri::AppHandle,
) -> Result<Applied, String> {
    import_apply(path, expected_hash, app, None).await
}

/// The body of `settings_import_apply`, also the COO's (behind its card):
/// with `agent` (its caller) every change goes through the same settings
/// setters as `settings.change` as the COO, never as the person, and each
/// failure's reason comes back redacted.
pub(crate) async fn import_apply(
    path: String,
    expected_hash: String,
    app: tauri::AppHandle,
    agent: Option<Caller>,
) -> Result<Applied, String> {
    on_blocking(move || {
        let file = read_file(&path)?;
        let redact = agent.is_some();
        let cx = cx_as(&app, agent);
        let (values, unreadable) = tauri::async_runtime::block_on(current(&cx));
        let preview = plan(&file, &values, &unreadable)?;
        if preview.hash != expected_hash {
            return Err("The file or your settings changed since the preview, so nothing was applied. Preview the import again.".into());
        }
        let mut applied = 0;
        let mut failed = Vec::new();
        for change in preview.changes {
            let result = tauri::async_runtime::block_on(app_actions::change_setting(
                cx.fresh(),
                &change.key,
                change.value,
            ));
            match result {
                Ok(_) => applied += 1,
                Err(why) => failed.push(Ignored {
                    key: change.key,
                    why: if redact {
                        crate::claude_runner::redact(&why)
                    } else {
                        why
                    },
                }),
            }
        }
        Ok(Applied { applied, failed })
    })
    .await
}

/// The most low-risk changes a COO import card lists one by one; a
/// confirm-tier change is always listed.
const CARD_LISTED: usize = 40;

/// The lines a COO import card shows for `preview`: every change old → new
/// (confirm-tier ones first, as the preview orders them), then what is left
/// alone. Pure.
pub(crate) fn card_lines(preview: &Preview) -> Vec<String> {
    let mut lines = vec![format!(
        "{} setting(s) change, {} already match.",
        preview.changes.len(),
        preview.unchanged
    )];
    let mut quiet = 0;
    for change in &preview.changes {
        if !change.asks && quiet >= CARD_LISTED {
            continue;
        }
        if !change.asks {
            quiet += 1;
        }
        lines.push(format!("{}: {} → {}", change.key, change.old, change.new));
    }
    let hidden = preview
        .changes
        .iter()
        .filter(|c| !c.asks)
        .count()
        .saturating_sub(CARD_LISTED);
    if hidden > 0 {
        lines.push(format!("and {hidden} more low-risk setting(s)"));
    }
    if preview.changes.iter().any(|c| c.asks) {
        lines.push(
            "Some of these change what agents may do, what runs on its own or what spends.".into(),
        );
    }
    if !preview.ignored.is_empty() {
        lines.push(format!(
            "{} key(s) in the file are ignored (secrets, unknown or personal ones).",
            preview.ignored.len()
        ));
    }
    lines
}

/// `settings.settings_import_apply`'s card: the preview as it is now, which
/// must still hash to the `expectedHash` the COO was given.
pub(crate) fn import_card(
    cx: &Cx,
    a: &app_actions::Args,
) -> Result<Option<app_actions::Card>, String> {
    let text = |key: &str| {
        a.0.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let (path, expected) = (text("path"), text("expectedHash"));
    let app = cx
        .host
        .app()
        .ok_or("Importing settings needs the running app.")?
        .clone();
    let file = read_file(&path)?;
    let (values, unreadable) =
        tauri::async_runtime::block_on(current(&cx_as(&app, Some(cx.caller.clone()))));
    let preview = for_agent(plan(&file, &values, &unreadable)?, &values);
    if preview.hash != expected {
        return Err("The file or the settings changed since the preview. Preview the import again. Nothing was applied.".into());
    }
    if preview.changes.is_empty() {
        return Err("Nothing in that file would change a setting. Nothing was applied.".into());
    }
    let mut card = app_actions::Card::new("Hollow app tools: import settings from a file")
        .line(format!("File: {path}"));
    for line in card_lines(&preview) {
        card = card.line(line);
    }
    Ok(Some(card))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current_of(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn a_plan_lists_real_changes_and_says_why_it_ignores_the_rest() {
        let file = json!({
            "hollowSettings": 1,
            "exportedAt": "2026-10-10T10:00:00Z",
            "settings": {
                "theme": "dark",
                "agents.defaultAgent": "codex",
                "agents.model.claude": "claude-opus-5-5",
                "jev.apiKey": "ts-live-123",
                "nope.unknown": true,
                "agents.codexNoOutputTimeoutMin": "soon",
            }
        });
        let current = current_of(&[
            ("theme", json!("light")),
            ("agents.defaultAgent", json!("codex")),
            ("agents.model.claude", json!(null)),
        ]);
        let preview = plan(&file, &current, &[]).unwrap();
        let changed: Vec<&str> = preview.changes.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(changed, ["agents.model.claude", "theme"]);
        assert_eq!(preview.unchanged, 1);
        let ignored: Vec<(&str, bool)> = preview
            .ignored
            .iter()
            .map(|i| (i.key.as_str(), !i.why.is_empty()))
            .collect();
        assert_eq!(
            ignored,
            [
                ("agents.codexNoOutputTimeoutMin", true),
                ("jev.apiKey", true),
                ("nope.unknown", true)
            ]
        );
        assert!(preview.ignored[1].why.contains("secret"));
        assert_eq!(preview.exported_at.as_deref(), Some("2026-10-10T10:00:00Z"));
        let theme = &preview.changes[1];
        assert!(
            theme.old.contains("light") && theme.new.contains("dark"),
            "{theme:?}"
        );
    }

    #[test]
    fn the_hash_binds_the_values_and_the_current_state() {
        let file = |theme: &str| json!({ "hollowSettings": 1, "settings": { "theme": theme } });
        let light = current_of(&[("theme", json!("light"))]);
        let a = plan(&file("dark"), &light, &[]).unwrap().hash;
        assert_eq!(a, plan(&file("dark"), &light, &[]).unwrap().hash, "stable");
        assert_ne!(
            a,
            plan(
                &file("light"),
                &current_of(&[("theme", json!("dark"))]),
                &[]
            )
            .unwrap()
            .hash
        );
        let other = current_of(&[("theme", json!("system"))]);
        assert_ne!(
            a,
            plan(&file("dark"), &other, &[]).unwrap().hash,
            "the current value moved"
        );
    }

    #[test]
    fn only_a_hollow_settings_file_is_read() {
        let empty = Map::new();
        assert!(plan(&json!({ "settings": {} }), &empty, &[])
            .unwrap_err()
            .contains("isn't a Hollow settings file"));
        assert!(plan(&json!({ "hollowSettings": 2, "settings": {} }), &empty, &[]).is_err());
        assert!(plan(&json!({ "hollowSettings": 1 }), &empty, &[])
            .unwrap_err()
            .contains("no \"settings\""));
        assert!(read_file("relative.json")
            .unwrap_err()
            .contains("file picker"));
    }

    #[test]
    fn an_export_never_holds_a_secret_and_round_trips() {
        let mut values = Map::new();
        for key in keys::KEYS {
            let personal = PERSONAL.iter().any(|(k, _)| *k == key.key);
            assert_eq!(
                exportable(key),
                !matches!(key.kind, Type::Secret) && !personal,
                "{}",
                key.key
            );
        }
        for (key, _) in PERSONAL {
            assert!(keys::find(key).is_some(), "{key} is still a setting");
        }
        values.insert("theme".into(), json!("dark"));
        let body = file_body(&values, "2026-10-10T10:00:00Z");
        assert_eq!(body["hollowSettings"], json!(1));
        let back = plan(&body, &current_of(&[("theme", json!("dark"))]), &[]).unwrap();
        assert!(back.changes.is_empty() && back.ignored.is_empty());
        assert_eq!(back.unchanged, 1);
        assert!(
            keys::KEYS.iter().any(|k| matches!(k.kind, Type::Secret)),
            "the registry still has a secret to keep out"
        );
    }

    #[test]
    fn defaults_personal_keys_and_unreadable_values_are_left_alone_with_a_reason() {
        let file = json!({ "hollowSettings": 1, "settings": {
            "agents.model.claude": null,
            "agents.model.codex": null,
            "paletteRecents": { "ids": ["vault:notes/a.md"] },
            "theme": "dark",
            "agentTools.app": true,
        }});
        let current = current_of(&[
            ("agents.model.claude", json!(null)),
            ("agents.model.codex", json!("gpt-6")),
            ("agentTools.app", json!(false)),
        ]);
        let preview = plan(&file, &current, &["theme".to_string()]).unwrap();
        assert!(preview.changes.is_empty(), "{:?}", preview.changes);
        assert_eq!(preview.unchanged, 1, "null over null matches");
        let why: Vec<(&str, &str)> = preview
            .ignored
            .iter()
            .map(|i| (i.key.as_str(), i.why.as_str()))
            .collect();
        assert!(
            why.iter()
                .any(|(k, w)| *k == "agents.model.codex" && w.contains("default")),
            "{why:?}"
        );
        assert!(
            why.iter()
                .any(|(k, w)| *k == "paletteRecents" && w.contains("personal")),
            "{why:?}"
        );
        assert!(
            why.iter()
                .any(|(k, w)| *k == "theme" && w.contains("couldn't read")),
            "{why:?}"
        );
        assert!(
            why.iter()
                .any(|(k, w)| *k == "agentTools.app" && w.contains("in Settings itself")),
            "{why:?}"
        );
    }

    #[test]
    fn shortcuts_travel_without_their_warnings_and_confirm_keys_come_first() {
        let shortcut = |warnings: bool| {
            let mut v = json!({ "enabled": true, "keys": { "palette": "Cmd+K" } });
            if warnings {
                v["warnings"] = json!(["Cmd+K is taken"]);
            }
            v
        };
        assert_eq!(portable("shortcuts", shortcut(true)), shortcut(false));
        let file = json!({ "hollowSettings": 1, "settings": {
            "shortcuts": shortcut(true),
            "theme": "dark",
            "heartbeat.enabled": true,
        }});
        let current = current_of(&[
            ("shortcuts", shortcut(false)),
            ("theme", json!("light")),
            ("heartbeat.enabled", json!(false)),
        ]);
        let preview = plan(&file, &current, &[]).unwrap();
        assert_eq!(preview.unchanged, 1, "only warnings differed");
        let order: Vec<(&str, bool)> = preview
            .changes
            .iter()
            .map(|c| (c.key.as_str(), c.asks))
            .collect();
        assert_eq!(order, [("heartbeat.enabled", true), ("theme", false)]);
    }
}
