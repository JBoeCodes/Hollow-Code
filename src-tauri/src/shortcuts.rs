//! Keyboard shortcut settings (Settings, Keyboard shortcuts).
//!
//! `{app data}/shortcuts.json` holds `{enabled, keys}`: whether Hollow's
//! shortcuts are on at all, and the user's changes from the defaults.
//! `keys[id]` is an accelerator (`CmdOrCtrl+Shift+N`) or `""` for none; a
//! command not in `keys` keeps its default. The webview's table
//! (`src/shortcuts.ts`) owns every command's label and default and matches
//! keystrokes; this side applies what only the backend can, at boot and on
//! every change: the menu bar's accelerators (`app_menu::ITEMS`), the
//! system-wide shortcuts (`GLOBALS`: quick capture and Import a source, which
//! open the main window's Add sheet since S5, and the Main agent) and the
//! tray menu's hints for them. A vitest checks the two default tables agree.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{
    menu::{MenuItem, MenuItemKind},
    AppHandle, Emitter, Manager, Runtime,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

pub const FILE: &str = "shortcuts.json";
/// Tells the webview the settings changed (an agent's change, or another window's).
pub const EVENT: &str = "shortcuts-changed";

/// The system-wide shortcuts: (command id, default, tray item id).
pub const GLOBALS: &[(&str, &str, &str)] = &[
    // Three modifiers (five asks A4): ⌘⇧T is every browser's Reopen closed
    // tab, and a system-wide key takes its chord from every app.
    ("quick-capture", "CmdOrCtrl+Alt+Shift+T", "quick"),
    ("quick-ingest", "CmdOrCtrl+Alt+Shift+I", "ingest"),
    // Brings Hollow forward on the Main agent (main agent M1). Three
    // modifiers, because a system-wide key takes the chord from every app.
    ("main-agent", "CmdOrCtrl+Alt+Shift+H", "main-agent"),
];

/// Keys a chord may end in. The webview records the same set.
const KEYS: &[&str] = &[
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "[",
    "]",
    "\\",
    ",",
    ".",
    "/",
    ";",
    "'",
    "-",
    "=",
    "`",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
];

/// Keys that are not accelerators but the webview handles itself, and the
/// one command each may belong to: the keymap card's bare `?` and Esc, which
/// stops a running turn.
const BARE: &[(&str, &str)] = &[("shortcuts", "?"), ("stop-run", "Escape")];

/// Every command a shortcut can run: the webview's `COMMANDS` ids (a vitest
/// holds the two lists together).
pub const COMMAND_IDS: &[&str] = &[
    "go-work",
    "go-review",
    "go-tasks",
    "go-library",
    "go-automations",
    "settings",
    "go-back",
    "go-forward",
    "prev-conversation",
    "next-conversation",
    "workspace-conversation",
    "jump",
    "search-chats",
    "new-conversation",
    "new-quick-chat",
    "new-workspace",
    "new-note",
    "new-task",
    "search",
    "search-alt",
    "toggle-sidebar",
    "toggle-side-pane",
    "main-drawer",
    "shortcuts",
    "stop-run",
    "dictate",
    "quick-capture",
    "quick-ingest",
    "main-agent",
];

/// Commands whose default chord came after people could set their own keys,
/// with that chord: a saved key that already took it wins
/// (`yield_new_defaults`).
const NEW_COMMAND_DEFAULTS: &[(&str, &str)] = &[
    ("go-review", "CmdOrCtrl+5"),
    // Search chats (L1, 11-12): the top bar's search icon.
    ("search-chats", "CmdOrCtrl+Shift+K"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShortcutSettings {
    /// Off: no Hollow shortcut works, in the window or system-wide.
    pub enabled: bool,
    /// Command id → accelerator, or "" for none. Only changed commands.
    pub keys: BTreeMap<String, String>,
    /// Go-to target id → its one key after `g` (five asks A4: the webview's
    /// `GO_TARGETS`), or "" for none. Only changed targets. The webview
    /// handles these keys itself; nothing here registers them.
    pub leader: BTreeMap<String, String>,
}

impl Default for ShortcutSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            keys: BTreeMap::new(),
            leader: BTreeMap::new(),
        }
    }
}

/// What a save returns: the settings and anything that could not be applied
/// (a system-wide shortcut another app already holds).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutsView {
    #[serde(flatten)]
    pub settings: ShortcutSettings,
    pub warnings: Vec<String>,
}

static LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
/// Registered system-wide shortcuts: (command id, shortcut).
static REGISTERED: Lazy<Mutex<Vec<(String, Shortcut)>>> = Lazy::new(|| Mutex::new(Vec::new()));
/// The last apply's warnings, so a reload of Settings still shows them.
static WARNINGS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));
/// Set at boot when the system-wide shortcuts may register: not in setup
/// mode (the overlays have no vault yet) and not in a review build with its
/// own app data (it would take the keys from the installed app).
static GLOBALS_ALLOWED: AtomicBool = AtomicBool::new(false);

pub fn allow_globals() {
    GLOBALS_ALLOWED.store(true, Ordering::SeqCst);
}

fn path() -> Result<std::path::PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(FILE))
}

/// The saved settings; the defaults when there is no file or it doesn't parse.
pub fn load() -> ShortcutSettings {
    path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str::<ShortcutSettings>(&raw).ok())
        .map(drop_reserved)
        .map(drop_retired_leaders)
        .map(yield_new_defaults)
        .filter(|s| validate(s).is_ok())
        .unwrap_or_default()
}

/// A file saved before five asks A4 may hold a chord macOS owns: that one
/// change goes (its command keeps its default), never the whole file.
pub(crate) fn drop_reserved(mut s: ShortcutSettings) -> ShortcutSettings {
    s.keys.retain(|id, chord| {
        !reserved(chord, id)
            && !system_wide_short(id, chord)
            && !LOCAL_ALT.contains(&chord.as_str())
    });
    s
}

/// A system-wide command's chord without all of ⌘⌥⇧ (P1-49): it would take
/// that chord from every app.
fn system_wide_short(id: &str, chord: &str) -> bool {
    !chord.is_empty()
        && GLOBALS.iter().any(|(g, _, _)| *g == id)
        && !chord.starts_with("CmdOrCtrl+Alt+Shift+")
}

/// The in-surface ⌥ chords (`src/localKeys.ts`; a vitest holds the lists
/// equal): selected text's Make a task and Ask. No command
/// may take one (P1-49): the window leaves them to their surface, and a
/// menu accelerator would take them from it.
pub const LOCAL_ALT: &[&str] = &["CmdOrCtrl+Alt+T", "CmdOrCtrl+Alt+A"];

/// A file saved before overnight 11-08 S3 may name a go-to target that was
/// cut (today, commitments, weekly-review, agenda, inbox). That entry goes.
/// The rest of the file stays. `validate` refuses an unknown target, and
/// `load` would otherwise throw the whole file away, including unrelated
/// chords and a shortcuts switch the person turned off.
pub(crate) fn drop_retired_leaders(mut s: ShortcutSettings) -> ShortcutSettings {
    s.leader
        .retain(|id, _| LEADER_TARGETS.iter().any(|(target, _)| target == id));
    s
}

/// A file saved before a go-to target existed may have given that target's
/// default key to another one (`history: "k"` before the commitments ledger
/// took `k`, overnight 10-11 A5): the new target starts with no key, never
/// the whole file dropped by `validate`. A command's new default chord does
/// the same (Review's ⌘5, P1-14): the person's own command keeps it.
pub(crate) fn yield_new_defaults(mut s: ShortcutSettings) -> ShortcutSettings {
    for (id, default) in NEW_COMMAND_DEFAULTS {
        if s.keys.contains_key(*id) {
            continue;
        }
        if s.keys
            .iter()
            .any(|(other, chord)| other != id && chord == default)
        {
            s.keys.insert((*id).to_string(), String::new());
        }
    }
    for (id, default) in LEADER_TARGETS {
        if s.leader.contains_key(*id) {
            continue;
        }
        if s.leader
            .iter()
            .any(|(other, key)| other != id && key == default)
        {
            s.leader.insert((*id).to_string(), String::new());
        }
    }
    s
}

fn save(s: &ShortcutSettings) -> Result<(), String> {
    let path = path()?;
    let body = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(&path, body.as_bytes())
        .map_err(|e| format!("Could not save {}: {e}", path.display()))
}

/// Every go-to target and its default key after `g` (five asks A4): the
/// webview's `GO_TARGETS`, in order (a vitest holds the two lists together).
pub const LEADER_TARGETS: &[(&str, &str)] = &[
    ("home", "h"),
    ("main-agent", "m"),
    ("ask-vault", "v"),
    // what's-new (n), runbooks (r) and map (w) were cut (overnight 11-08 S4).
    ("tasks", "t"),
    ("library", "l"),
    ("automations", "a"),
    ("mission-control", "o"),
    ("canvas-gallery", "x"),
    ("waiting", "b"),
    ("history", "p"),
    ("usage", "u"),
    ("settings", "s"),
    ("quick-chat", "q"),
    ("jump", "j"),
];

/// macOS's own chords, which no command may take (five asks A4): the
/// webview's `MAC_RESERVED` (a vitest holds the two lists together), so an
/// agent's `change_setting` is refused like Settings' recorder. F-keys with
/// any modifiers are refused by rule in [`reserved`].
pub const MAC_RESERVED: &[&str] = &[
    "CmdOrCtrl+Q",
    "CmdOrCtrl+W",
    "CmdOrCtrl+H",
    "CmdOrCtrl+M",
    "CmdOrCtrl+Tab",
    "CmdOrCtrl+Space",
    "CmdOrCtrl+`",
    "CmdOrCtrl+,",
    "CmdOrCtrl+C",
    "CmdOrCtrl+V",
    "CmdOrCtrl+X",
    "CmdOrCtrl+Z",
    "CmdOrCtrl+Shift+Z",
    "CmdOrCtrl+A",
    "CmdOrCtrl+S",
    "CmdOrCtrl+F",
    "CmdOrCtrl+G",
    "CmdOrCtrl+Shift+G",
    "CmdOrCtrl+P",
    "CmdOrCtrl+N",
    "CmdOrCtrl+O",
    "CmdOrCtrl+T",
    "CmdOrCtrl+R",
    "CmdOrCtrl+Alt+H",
    "CmdOrCtrl+Alt+M",
    "CmdOrCtrl+Alt+Escape",
    "CmdOrCtrl+Alt+D",
    "CmdOrCtrl+Shift+3",
    "CmdOrCtrl+Shift+4",
    "CmdOrCtrl+Shift+5",
    "CmdOrCtrl+Shift+/",
    "CmdOrCtrl+?",
    "CmdOrCtrl+Shift+Q",
    "CmdOrCtrl+Alt+Shift+Q",
    "CmdOrCtrl+Alt+Shift+V",
    "CmdOrCtrl+Alt+Shift+Escape",
    // Accessibility Zoom, Cancel and the Touch Bar screenshot (P1-49).
    "CmdOrCtrl+Alt+8",
    "CmdOrCtrl+Alt+=",
    "CmdOrCtrl+Alt+-",
    "CmdOrCtrl+Alt+\\",
    "CmdOrCtrl+.",
    "CmdOrCtrl+Shift+6",
    "CmdOrCtrl+ArrowLeft",
    "CmdOrCtrl+ArrowRight",
    "CmdOrCtrl+ArrowUp",
    "CmdOrCtrl+ArrowDown",
];

/// A reserved chord Hollow uses for its macOS meaning, and the one command
/// that may hold it (the webview's `STANDARD_MEANING`).
pub const STANDARD_MEANING: &[(&str, &str)] = &[
    ("CmdOrCtrl+,", "settings"),
    ("CmdOrCtrl+N", "new-conversation"),
];

/// Whether command `id` may not have `chord` because macOS owns it.
pub fn reserved(chord: &str, id: &str) -> bool {
    let key = chord.rsplit('+').next().unwrap_or("");
    let fkey = key
        .strip_prefix('F')
        .is_some_and(|n| matches!(n.parse::<u8>(), Ok(1..=12)));
    if chord.starts_with("CmdOrCtrl+") && fkey {
        return true;
    }
    MAC_RESERVED.contains(&chord)
        && !STANDARD_MEANING
            .iter()
            .any(|(c, owner)| *c == chord && *owner == id)
}

/// A go-to target's key in effect: its change, else its default.
fn leader_key_of<'a>(s: &'a ShortcutSettings, id: &'a str) -> &'a str {
    s.leader.get(id).map(String::as_str).unwrap_or_else(|| {
        LEADER_TARGETS
            .iter()
            .find(|(t, _)| *t == id)
            .map(|(_, k)| *k)
            .unwrap_or("")
    })
}

/// Whether `key` may follow `g`: one lowercase letter or digit, or "".
pub fn leader_key_ok(key: &str) -> bool {
    key.is_empty()
        || (key.len() == 1
            && key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

/// Whether `chord` is one a command may have: "" (none), a bare key the
/// webview handles, or ⌘ (Ctrl elsewhere), optionally ⌥ and ⇧, then a key.
pub fn chord_ok(chord: &str) -> bool {
    if chord.is_empty() || BARE.iter().any(|(_, key)| *key == chord) {
        return true;
    }
    let Some(rest) = chord.strip_prefix("CmdOrCtrl+") else {
        return false;
    };
    let rest = rest.strip_prefix("Alt+").unwrap_or(rest);
    let key = rest.strip_prefix("Shift+").unwrap_or(rest);
    KEYS.contains(&key)
}

pub fn validate(s: &ShortcutSettings) -> Result<(), String> {
    if s.keys.len() > 64 {
        return Err("At most 64 shortcuts.".into());
    }
    for (id, key) in &s.leader {
        if !LEADER_TARGETS.iter().any(|(t, _)| t == id) {
            return Err(format!(
                "`{id}` is not a go-to target. The targets: {}.",
                LEADER_TARGETS
                    .iter()
                    .map(|(t, _)| *t)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !leader_key_ok(key) {
            return Err(format!(
                "{id}: `{key}` can't follow g. Use one lowercase letter or digit, or \"\" for none."
            ));
        }
    }
    // Collisions among the keys in effect, defaults included: a change that
    // takes another target's default key must clear that target too.
    let mut taken: BTreeMap<&str, &str> = BTreeMap::new();
    for (id, _) in LEADER_TARGETS {
        let key = leader_key_of(s, id);
        if key.is_empty() {
            continue;
        }
        if let Some(other) = taken.insert(key, id) {
            return Err(format!(
                "g then {key} is set for both {other} and {id}; give one of them another key or \"\"."
            ));
        }
    }
    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    for (id, chord) in &s.keys {
        if !COMMAND_IDS.contains(&id.as_str()) {
            return Err(format!(
                "`{id}` is not a shortcut command. The commands: {}.",
                COMMAND_IDS.join(", ")
            ));
        }
        if !chord_ok(chord) {
            return Err(format!("{id}: `{chord}` is not a shortcut. Use ⌘ with an optional ⌥ and ⇧ and one key, written like CmdOrCtrl+Shift+K, or \"\" for none."));
        }
        if reserved(chord, id) {
            return Err(format!(
                "{id}: `{chord}` is one of macOS's own shortcuts. Pick another key."
            ));
        }
        if LOCAL_ALT.contains(&chord.as_str()) {
            return Err(format!(
                "{id}: `{chord}` belongs to selected text (make a task, ask, make a canvas). Pick another key."
            ));
        }
        if system_wide_short(id, chord) {
            return Err(format!(
                "{id} works in every app, so it needs ⌥⇧⌘ and a key (CmdOrCtrl+Alt+Shift+…), not `{chord}`."
            ));
        }
        if let Some((owner, _)) = BARE.iter().find(|(_, key)| *key == chord) {
            if owner != id {
                return Err(format!(
                    "{id}: `{chord}` belongs to {owner} only; other commands need ⌘."
                ));
            }
        }
        if !chord.is_empty() {
            if let Some(other) = seen.insert(chord.as_str(), id.as_str()) {
                return Err(format!("{chord} is set for both {other} and {id}."));
            }
        }
    }
    Ok(())
}

/// The accelerator a command has now: none when shortcuts are off, the
/// command is cleared, or its key is one the webview handles itself.
pub fn effective(s: &ShortcutSettings, id: &str, default: Option<&str>) -> Option<String> {
    if !s.enabled {
        return None;
    }
    let chord = s.keys.get(id).map(String::as_str).or(default)?;
    chord.starts_with("CmdOrCtrl+").then(|| chord.to_string())
}

/// The tray menu's quick capture / quick ingest items, so their key hints follow.
pub struct TrayItems<R: Runtime>(pub Vec<(&'static str, MenuItem<R>)>);

pub(crate) fn menu_item<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<MenuItem<R>> {
    for top in app.menu()?.items().ok()? {
        if let MenuItemKind::Submenu(sub) = top {
            if let Some(MenuItemKind::MenuItem(item)) = sub.get(id) {
                return Some(item);
            }
        }
    }
    None
}

/// Put `s` into effect: menu accelerators, system-wide shortcuts, tray hints.
/// Returns what could not be applied.
pub fn apply<R: Runtime>(app: &AppHandle<R>, s: &ShortcutSettings) -> Vec<String> {
    let mut warnings = Vec::new();
    for (id, label, default) in crate::app_menu::ITEMS {
        if let Some(item) = menu_item(app, id) {
            if let Err(e) = item.set_accelerator(effective(s, id, *default)) {
                warnings.push(format!("{label}: the menu bar refused the shortcut ({e})."));
            }
        }
    }
    let mut live: Vec<String> = Vec::new();
    if GLOBALS_ALLOWED.load(Ordering::SeqCst) {
        // Never hold REGISTERED while (un)registering: that waits on the main
        // thread, where a key press's handler takes the same lock.
        let old: Vec<(String, Shortcut)> =
            std::mem::take(&mut *REGISTERED.lock().unwrap_or_else(|e| e.into_inner()));
        for (_, shortcut) in old {
            let _ = app.global_shortcut().unregister(shortcut);
        }
        let mut registered = Vec::new();
        for (id, default, _) in GLOBALS {
            let Some(chord) = effective(s, id, Some(default)) else {
                continue;
            };
            let registered_ok = chord
                .parse::<Shortcut>()
                .map_err(|e| e.to_string())
                .and_then(|shortcut| {
                    app.global_shortcut()
                        .register(shortcut)
                        .map(|_| shortcut)
                        .map_err(|e| e.to_string())
                });
            match registered_ok {
                Ok(shortcut) => registered.push((id.to_string(), shortcut)),
                Err(e) => warnings.push(format!(
                    "{chord} for {} could not be set system-wide; another app may use it ({e}).",
                    id.replace('-', " ")
                )),
            }
        }
        live = registered.iter().map(|(id, _)| id.clone()).collect();
        *REGISTERED.lock().unwrap_or_else(|e| e.into_inner()) = registered;
    }
    // The tray hints only keys that work now (P1-49).
    if let Some(tray) = app.try_state::<TrayItems<R>>() {
        for (tray_id, item) in &tray.0 {
            let Some((id, default, _)) = GLOBALS.iter().find(|(_, _, t)| t == tray_id) else {
                continue;
            };
            let _ = item.set_accelerator(tray_hint(s, id, default, &live));
        }
    }
    *WARNINGS.lock().unwrap_or_else(|e| e.into_inner()) = warnings.clone();
    warnings
}

/// A tray item's key hint: its chord only when it registered system-wide,
/// never one that is off, cleared, held by another app or not set in this
/// build (P1-49).
pub fn tray_hint(
    s: &ShortcutSettings,
    id: &str,
    default: &str,
    registered: &[String],
) -> Option<String> {
    effective(s, id, Some(default)).filter(|_| registered.iter().any(|r| r == id))
}

/// One notification for the system-wide keys that failed at launch, all
/// of them in its body (P1-49); none when every key registered.
pub fn boot_notice(warnings: &[String]) -> Option<crate::notifications::NotificationInput> {
    if warnings.is_empty() {
        return None;
    }
    Some(crate::notifications::NotificationInput {
        title: if warnings.len() == 1 {
            "A keyboard shortcut didn't start".into()
        } else {
            format!("{} keyboard shortcuts didn't start", warnings.len())
        },
        body: warnings.join(" "),
        kind: "warning".into(),
        source: Some("shortcuts".into()),
        action_label: Some("Keyboard shortcuts".into()),
        action_url: Some("settings:shortcuts".into()),
        notify_native: false,
    })
}

/// Which system-wide command a pressed shortcut is (the plugin's handler).
pub fn global_for(shortcut: &Shortcut) -> Option<String> {
    REGISTERED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(_, s)| s == shortcut)
        .map(|(id, _)| id.clone())
}

pub fn view() -> ShortcutsView {
    ShortcutsView {
        settings: load(),
        warnings: WARNINGS.lock().unwrap_or_else(|e| e.into_inner()).clone(),
    }
}

/// Validate, save and apply, then tell the webview. Settings and the
/// `shortcuts` setting agents change (`settings_keys.rs`) both come here.
pub fn change<R: Runtime>(
    app: Option<&AppHandle<R>>,
    next: ShortcutSettings,
) -> Result<ShortcutsView, String> {
    validate(&next)?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    save(&next)?;
    let warnings = app.map(|app| apply(app, &next)).unwrap_or_default();
    let view = ShortcutsView {
        settings: next,
        warnings,
    };
    if let Some(app) = app {
        let _ = app.emit(EVENT, &view);
    }
    Ok(view)
}

/// An agent's change (`change_setting shortcuts`): `enabled` if given, and
/// `keys` merged into the saved ones, where `null` puts a command back on
/// its default.
pub fn merge(
    current: ShortcutSettings,
    patch: &serde_json::Value,
) -> Result<ShortcutSettings, String> {
    let Some(patch) = patch.as_object() else {
        return Err("shortcuts: the value must be {enabled?, keys?, leader?}.".into());
    };
    let mut next = current;
    if let Some(on) = patch.get("enabled") {
        next.enabled = on
            .as_bool()
            .ok_or("shortcuts.enabled must be true or false.")?;
    }
    if let Some(keys) = patch.get("keys") {
        let keys = keys
            .as_object()
            .ok_or("shortcuts.keys must be {command id: chord, \"\" or null}.")?;
        for (id, chord) in keys {
            match chord {
                serde_json::Value::Null => {
                    next.keys.remove(id);
                }
                serde_json::Value::String(c) => {
                    next.keys.insert(id.clone(), c.clone());
                }
                _ => {
                    return Err(format!(
                        "shortcuts.keys.{id} must be a chord, \"\" or null."
                    ))
                }
            }
        }
    }
    if let Some(leader) = patch.get("leader") {
        let leader = leader.as_object().ok_or(
            "shortcuts.leader must be {go-to target id: one letter or digit, \"\" or null}.",
        )?;
        for (id, key) in leader {
            match key {
                serde_json::Value::Null => {
                    next.leader.remove(id);
                }
                serde_json::Value::String(k) => {
                    next.leader.insert(id.clone(), k.clone());
                }
                _ => {
                    return Err(format!(
                        "shortcuts.leader.{id} must be a key, \"\" or null."
                    ))
                }
            }
        }
    }
    for key in patch.keys() {
        if key != "enabled" && key != "keys" && key != "leader" {
            return Err(format!(
                "shortcuts has no field `{key}` (only enabled, keys and leader)."
            ));
        }
    }
    Ok(next)
}

// The commands that apply are async, so they run off the main thread: the
// menu and global-shortcut calls wait on it, and a sync command would hold
// LOCK on it while an agent's change waits there too.

#[tauri::command]
pub fn get_shortcut_settings() -> ShortcutsView {
    view()
}

#[tauri::command]
pub async fn set_shortcut_settings(
    app: AppHandle,
    settings: ShortcutSettings,
) -> Result<ShortcutsView, String> {
    change(Some(&app), settings)
}

/// While Settings records a key: every shortcut off without saving (so the
/// menu bar and the system-wide keys don't take the keystroke first), then
/// the saved settings back.
#[tauri::command]
pub async fn suspend_shortcuts(app: AppHandle, suspended: bool) -> Vec<String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if suspended {
        apply(
            &app,
            &ShortcutSettings {
                enabled: false,
                ..Default::default()
            },
        );
        Vec::new()
    } else {
        apply(&app, &load())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_targets_default_yields_to_a_key_the_person_already_gave_away() {
        // Saved when history took `t`, which Tasks uses by default: the file
        // still loads whole (shortcuts stay off), history keeps `t`, and
        // Tasks has no key.
        let mut s = ShortcutSettings::default();
        s.leader.insert("history".into(), "t".into());
        s.enabled = false;
        assert!(validate(&s).is_err(), "as saved, it collides");
        let s = yield_new_defaults(s);
        assert!(validate(&s).is_ok());
        assert!(!s.enabled);
        assert_eq!(leader_key_of(&s, "tasks"), "");
        assert_eq!(leader_key_of(&s, "history"), "t");
        // Nothing changes for a file without the collision.
        let plain = yield_new_defaults(ShortcutSettings::default());
        assert!(plain.leader.is_empty());
        assert_eq!(leader_key_of(&plain, "tasks"), "t");
    }

    #[test]
    fn a_new_commands_default_chord_yields_to_one_the_person_already_gave_away() {
        // Saved before Review took ⌘5 (P1-14): the person's command keeps
        // ⌘5 and go-review starts with none, so one key never runs two.
        let mut s = ShortcutSettings::default();
        s.keys.insert("new-task".into(), "CmdOrCtrl+5".into());
        let s = yield_new_defaults(s);
        assert!(validate(&s).is_ok());
        assert_eq!(s.keys.get("go-review").map(String::as_str), Some(""));
        assert_eq!(effective(&s, "go-review", Some("CmdOrCtrl+5")), None);
        assert_eq!(
            effective(&s, "new-task", None).as_deref(),
            Some("CmdOrCtrl+5")
        );
        // A person who set go-review themselves keeps their choice.
        let mut own = ShortcutSettings::default();
        own.keys
            .insert("go-review".into(), "CmdOrCtrl+Shift+R".into());
        own.keys.insert("new-task".into(), "CmdOrCtrl+5".into());
        let own = yield_new_defaults(own);
        assert_eq!(
            own.keys.get("go-review").map(String::as_str),
            Some("CmdOrCtrl+Shift+R")
        );
        // Without the collision nothing changes.
        assert!(yield_new_defaults(ShortcutSettings::default())
            .keys
            .is_empty());
    }

    #[test]
    fn chords_are_command_chords_or_the_webviews_bare_keys() {
        for ok in [
            "",
            "?",
            "Escape",
            "CmdOrCtrl+N",
            "CmdOrCtrl+Shift+[",
            "CmdOrCtrl+Alt+Shift+F5",
            "CmdOrCtrl+\\",
            "CmdOrCtrl+ArrowUp",
            "CmdOrCtrl+,",
        ] {
            assert!(chord_ok(ok), "{ok}");
        }
        for bad in [
            "N",
            "Shift+N",
            "CmdOrCtrl+",
            "CmdOrCtrl+Enter",
            "CmdOrCtrl+Shift+Alt+N",
            "Ctrl+N",
            "CmdOrCtrl+NN",
            "?x",
        ] {
            assert!(!chord_ok(bad), "{bad}");
        }
    }

    #[test]
    fn every_chord_the_table_allows_parses_as_a_menu_and_global_accelerator() {
        for key in KEYS {
            for prefix in [
                "CmdOrCtrl+",
                "CmdOrCtrl+Shift+",
                "CmdOrCtrl+Alt+",
                "CmdOrCtrl+Alt+Shift+",
            ] {
                let chord = format!("{prefix}{key}");
                assert!(chord_ok(&chord), "{chord}");
                assert!(
                    chord.parse::<Shortcut>().is_ok(),
                    "global shortcut parser refuses {chord}"
                );
            }
        }
        for (_, default, _) in GLOBALS {
            assert!(
                chord_ok(default) && default.parse::<Shortcut>().is_ok(),
                "{default}"
            );
        }
        for (id, _, default) in crate::app_menu::ITEMS {
            if let Some(d) = default {
                assert!(chord_ok(d), "menu default for {id}: {d}");
            }
        }
    }

    #[test]
    fn validate_refuses_bad_ids_bad_chords_and_one_chord_twice() {
        let mut s = ShortcutSettings::default();
        s.keys.insert("new-note".into(), "CmdOrCtrl+J".into());
        s.keys.insert("search".into(), "".into());
        s.keys.insert("go-back".into(), "".into());
        assert!(validate(&s).is_ok(), "two cleared commands are not a clash");
        s.keys.insert("new-task".into(), "CmdOrCtrl+J".into());
        assert!(validate(&s).unwrap_err().contains("both"));
        let mut s = ShortcutSettings::default();
        s.keys.insert("New Note".into(), "CmdOrCtrl+J".into());
        assert!(validate(&s).is_err());
        let mut s = ShortcutSettings::default();
        s.keys.insert("copy-build".into(), "CmdOrCtrl+J".into());
        assert!(
            validate(&s).unwrap_err().contains("not a shortcut command"),
            "only Settings' commands"
        );
        let mut s = ShortcutSettings::default();
        s.keys.insert("go-work".into(), "Escape".into());
        assert!(validate(&s).unwrap_err().contains("stop-run only"));
        s.keys.clear();
        s.keys.insert("stop-run".into(), "Escape".into());
        s.keys.insert("shortcuts".into(), "?".into());
        assert!(validate(&s).is_ok());
        let mut s = ShortcutSettings::default();
        s.keys.insert("new-note".into(), "J".into());
        assert!(validate(&s).is_err());
    }

    #[test]
    fn validate_refuses_macos_shortcuts_except_their_own_meaning() {
        for (id, chord) in [
            ("search", "CmdOrCtrl+Q"),
            ("new-note", "CmdOrCtrl+N"),
            ("go-work", "CmdOrCtrl+Shift+3"),
            ("go-tasks", "CmdOrCtrl+Alt+Shift+F5"),
            ("go-back", "CmdOrCtrl+ArrowLeft"),
        ] {
            let mut s = ShortcutSettings::default();
            s.keys.insert(id.into(), chord.into());
            assert!(validate(&s).unwrap_err().contains("macOS"), "{id} {chord}");
        }
        let mut s = ShortcutSettings::default();
        s.keys.insert("settings".into(), "CmdOrCtrl+,".into());
        s.keys
            .insert("new-conversation".into(), "CmdOrCtrl+N".into());
        assert!(validate(&s).is_ok(), "their own macOS meaning");
        // An agent can rebind go-to keys too.
        let next = merge(
            ShortcutSettings::default(),
            &serde_json::json!({"leader": {"settings": "9", "tasks": null}}),
        )
        .unwrap();
        assert_eq!(next.leader.get("settings").map(String::as_str), Some("9"));
        assert!(merge(
            ShortcutSettings::default(),
            &serde_json::json!({"leader": {"settings": 3}})
        )
        .is_err());
    }

    #[test]
    fn an_older_file_with_a_macos_chord_keeps_everything_else() {
        let mut s = ShortcutSettings {
            enabled: false,
            ..ShortcutSettings::default()
        };
        s.keys.insert("go-work".into(), "CmdOrCtrl+R".into());
        s.keys.insert("search".into(), "CmdOrCtrl+Shift+K".into());
        let kept = drop_reserved(s);
        assert!(validate(&kept).is_ok());
        assert!(!kept.enabled, "the switch survives");
        assert_eq!(kept.keys.len(), 1);
        assert!(kept.keys.contains_key("search"));
    }

    // P1-49: a system-wide key takes its chord from every app, so it needs
    // all three of ⌘⌥⇧; and macOS Zoom, Cancel and the Touch Bar screenshot
    // are refused for every command.
    #[test]
    fn system_wide_keys_need_three_modifiers_and_zoom_chords_are_refused() {
        for chord in ["CmdOrCtrl+K", "CmdOrCtrl+Alt+K", "CmdOrCtrl+Shift+K"] {
            let mut s = ShortcutSettings::default();
            s.keys.insert("quick-capture".into(), chord.into());
            assert!(validate(&s).unwrap_err().contains("⌥⇧⌘"), "{chord}");
        }
        let mut s = ShortcutSettings::default();
        s.keys
            .insert("quick-capture".into(), "CmdOrCtrl+Alt+Shift+K".into());
        s.keys.insert("search".into(), "CmdOrCtrl+Shift+K".into());
        assert!(validate(&s).is_ok());
        for chord in [
            "CmdOrCtrl+Alt+8",
            "CmdOrCtrl+Alt+=",
            "CmdOrCtrl+Alt+-",
            "CmdOrCtrl+Alt+\\",
            "CmdOrCtrl+.",
            "CmdOrCtrl+Shift+6",
        ] {
            for (id, _) in GLOBALS
                .iter()
                .map(|(id, d, _)| (*id, *d))
                .chain([("jump", "")])
            {
                assert!(reserved(chord, id), "{id} {chord}");
            }
        }
        // Selected text's ⌥⌘T / ⌥⌘A are no command's.
        for chord in LOCAL_ALT {
            let mut s = ShortcutSettings::default();
            s.keys.insert("jump".into(), (*chord).into());
            assert!(
                validate(&s).unwrap_err().contains("selected text"),
                "{chord}"
            );
        }
        // An older file with ⌘K for quick capture, or ⌥⌘T for the palette,
        // keeps everything else.
        let mut old = ShortcutSettings::default();
        old.keys.insert("search".into(), "CmdOrCtrl+Alt+T".into());
        old.keys
            .insert("quick-capture".into(), "CmdOrCtrl+K".into());
        old.keys.insert("jump".into(), "CmdOrCtrl+Alt+J".into());
        let kept = drop_reserved(old);
        assert!(validate(&kept).is_ok());
        assert_eq!(kept.keys.len(), 1);
        assert!(kept.keys.contains_key("jump"));
    }

    #[test]
    fn the_tray_hints_only_keys_that_registered() {
        let s = ShortcutSettings::default();
        let registered = vec!["quick-capture".to_string()];
        assert_eq!(
            tray_hint(&s, "quick-capture", "CmdOrCtrl+Alt+Shift+T", &registered),
            Some("CmdOrCtrl+Alt+Shift+T".to_string())
        );
        assert_eq!(
            tray_hint(&s, "quick-ingest", "CmdOrCtrl+Alt+Shift+I", &registered),
            None,
            "not registered (off, another app has it, or a review build)"
        );
    }

    #[test]
    fn boot_failures_post_one_notification() {
        assert!(boot_notice(&[]).is_none());
        let n = boot_notice(&["a could not be set".into(), "b could not be set".into()]).unwrap();
        assert!(n.body.contains("a could not be set") && n.body.contains("b could not be set"));
        assert_eq!(n.action_url.as_deref(), Some("settings:shortcuts"));
    }

    #[test]
    fn validate_checks_the_go_to_keys() {
        let mut s = ShortcutSettings::default();
        s.leader.insert("tasks".into(), "z".into());
        s.leader.insert("home".into(), "".into());
        assert!(validate(&s).is_ok());
        s.leader.insert("library".into(), "z".into());
        assert!(validate(&s).unwrap_err().contains("both"));
        // Another target's default key, without clearing that target.
        let mut s = ShortcutSettings::default();
        s.leader.insert("tasks".into(), "h".into());
        assert!(validate(&s).unwrap_err().contains("both home and tasks"));
        s.leader.insert("home".into(), "".into());
        assert!(validate(&s).is_ok());
        let mut s = ShortcutSettings::default();
        s.leader.insert("tasks".into(), "T".into());
        assert!(validate(&s).unwrap_err().contains("can't follow g"));
        let mut s = ShortcutSettings::default();
        s.leader.insert("tasks".into(), "xy".into());
        assert!(validate(&s).is_err());
        let mut s = ShortcutSettings::default();
        s.leader.insert("nowhere".into(), "z".into());
        assert!(validate(&s).unwrap_err().contains("not a go-to target"));
        // A saved binding for a cut target must not reset the rest of the file.
        let mut old = ShortcutSettings {
            enabled: false,
            ..Default::default()
        };
        old.keys
            .insert("new-task".into(), "CmdOrCtrl+Shift+9".into());
        old.leader.insert("today".into(), "d".into());
        old.leader.insert("agenda".into(), "c".into());
        old.leader.insert("whats-new".into(), "n".into());
        old.leader.insert("runbooks".into(), "r".into());
        old.leader.insert("map".into(), "w".into());
        old.leader.insert("tasks".into(), "z".into());
        let kept = drop_retired_leaders(old);
        assert!(validate(&kept).is_ok());
        assert!(!kept.leader.contains_key("today"));
        assert!(!kept.leader.contains_key("agenda"));
        assert!(!kept.leader.contains_key("whats-new"));
        assert!(!kept.leader.contains_key("runbooks"));
        assert!(!kept.leader.contains_key("map"));
        assert_eq!(kept.leader.get("tasks").map(String::as_str), Some("z"));
        assert!(!kept.enabled);
        assert_eq!(
            kept.keys.get("new-task").map(String::as_str),
            Some("CmdOrCtrl+Shift+9")
        );
        // Drop the cut target before yielding, so a dead binding that held
        // Tasks' default key does not clear Tasks.
        let mut held = ShortcutSettings::default();
        held.leader.insert("today".into(), "t".into());
        let held = yield_new_defaults(drop_retired_leaders(held));
        assert!(validate(&held).is_ok());
        assert!(!held.leader.contains_key("today"));
        assert_eq!(leader_key_of(&held, "tasks"), "t");
        // Older files without the field still load.
        let old: ShortcutSettings = serde_json::from_str(r#"{"enabled":true,"keys":{}}"#).unwrap();
        assert!(old.leader.is_empty());
    }

    #[test]
    fn effective_follows_the_switch_the_override_and_the_default() {
        let mut s = ShortcutSettings::default();
        assert_eq!(
            effective(&s, "new-note", Some("CmdOrCtrl+Shift+N")).as_deref(),
            Some("CmdOrCtrl+Shift+N")
        );
        assert_eq!(effective(&s, "new-task", None), None);
        s.keys.insert("new-task".into(), "CmdOrCtrl+Shift+K".into());
        s.keys.insert("new-note".into(), "".into());
        s.keys.insert("shortcuts".into(), "?".into());
        assert_eq!(
            effective(&s, "new-task", None).as_deref(),
            Some("CmdOrCtrl+Shift+K")
        );
        assert_eq!(
            effective(&s, "new-note", Some("CmdOrCtrl+Shift+N")),
            None,
            "cleared"
        );
        assert_eq!(
            effective(&s, "shortcuts", None),
            None,
            "a bare key is never a menu accelerator"
        );
        s.enabled = false;
        assert_eq!(effective(&s, "new-task", None), None);
        assert_eq!(
            effective(&s, "settings", Some("CmdOrCtrl+,")),
            None,
            "off means every shortcut"
        );
    }

    #[test]
    fn an_agents_change_merges_into_the_saved_settings() {
        let mut current = ShortcutSettings::default();
        current.keys.insert("new-note".into(), "CmdOrCtrl+J".into());
        current.keys.insert("search".into(), "".into());
        let next = merge(
            current.clone(),
            &serde_json::json!({ "keys": { "search": null, "new-task": "CmdOrCtrl+Shift+K" } }),
        )
        .unwrap();
        assert!(next.enabled, "a change without enabled keeps it");
        assert_eq!(
            next.keys.get("new-note").map(String::as_str),
            Some("CmdOrCtrl+J")
        );
        assert!(
            !next.keys.contains_key("search"),
            "null puts it back on its default"
        );
        assert_eq!(
            next.keys.get("new-task").map(String::as_str),
            Some("CmdOrCtrl+Shift+K")
        );
        assert!(
            !merge(current.clone(), &serde_json::json!({ "enabled": false }))
                .unwrap()
                .enabled
        );
        assert!(merge(current.clone(), &serde_json::json!({ "enabled": "no" })).is_err());
        assert!(merge(
            current.clone(),
            &serde_json::json!({ "keys": { "search": 3 } })
        )
        .is_err());
        assert!(merge(current, &serde_json::json!({ "bindings": {} })).is_err());
    }

    #[test]
    fn settings_save_load_and_fall_back_to_the_defaults() {
        let dir = crate::agent_settings::app_data_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = ShortcutSettings {
            enabled: false,
            ..Default::default()
        };
        s.keys.insert("new-note".into(), "CmdOrCtrl+Alt+N".into());
        let view = change::<tauri::Wry>(None, s.clone()).unwrap();
        assert_eq!(view.settings, s);
        assert_eq!(load(), s);
        assert!(change::<tauri::Wry>(
            None,
            ShortcutSettings {
                keys: [("x".to_string(), "Q".to_string())].into(),
                ..Default::default()
            }
        )
        .is_err());
        assert_eq!(load(), s, "a refused change leaves the file alone");
        std::fs::write(dir.join(FILE), "{not json").unwrap();
        assert_eq!(load(), ShortcutSettings::default());
        let _ = std::fs::remove_file(dir.join(FILE));
    }
}
