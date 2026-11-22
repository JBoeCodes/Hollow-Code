//! This Mac's screen (Cua CU6, part 1): which apps an agent may look at and
//! act on, and the `bounded` manifest the standalone `CuaDriver.app` starts
//! with. Policy only; the broker (`computer_broker.rs`) applies it.
//!
//! Design: `outputs/cua-computer-use-design-2026-11-15.md`, section 3 ("This
//! Mac's real screen") and 4. The person's grants are the boundary: a run's
//! Driver is started with a manifest naming exactly the apps (and whether the
//! whole display) that run has been allowed, so a Driver that ignored Hollow's
//! checks still could not reach another app. A new grant restarts the run's
//! Driver with a wider manifest, and an app the person blocks later drops out of it.
//!
//! **Unverified until CU0 and the Driver's install** (Alex's step): the
//! names of the arguments that carry the app (`APP_ID_KEYS`, `APP_NAME_KEYS`)
//! and the manifest's field names and flags (`Manifest::to_json`,
//! `ThisMacLaunch`). They are one place each, so confirming them against the
//! Driver's schema is a one-file edit.

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The arguments that name the app a call targets. A call on this Mac must
/// carry one (the broker's tool descriptions say so).
pub(crate) const APP_ID_KEYS: &[&str] = &["bundle_id", "bundleId"];
pub(crate) const APP_NAME_KEYS: &[&str] = &["app", "app_name"];
/// Words that mark a key as aiming a call at an app, process or window.
const AIM_WORDS: &[&str] = &[
    "app", "bundle", "target", "process", "pid", "window", "title", "owner", "exe", "name",
];

/// Other ways to aim a call at a process or window. On This Mac they are
/// refused: Hollow checked the app it was told about, and only that one may
/// be named. (The real Driver may need them; that is for the install to
/// confirm, with the manifest enforcing the apps on the Driver's side.)
pub(crate) const TARGET_KEYS: &[&str] = &[
    "pid",
    "process_id",
    "process",
    "window_id",
    "windowId",
    "window",
    "window_title",
];

/// The person's edits to the default deny list: apps they chose to allow.
pub(crate) const POLICY_FILE: &str = "computer-this-mac.json";

/// One default-denied app (or family): the label Settings shows, its
/// lowercase bundle ids (a sub bundle follows its app), bundle id families
/// (prefixes) and its display names.
pub(crate) struct DenyGroup {
    pub label: &'static str,
    pub ids: &'static [&'static str],
    pub prefixes: &'static [&'static str],
    pub names: &'static [&'static str],
}

/// Apps denied until the person allows them (design section 4): terminals,
/// System Settings, Keychain Access, password managers and Hollow itself.
pub(crate) const DENY_GROUPS: &[DenyGroup] = &[
    DenyGroup {
        label: "Terminal",
        ids: &["com.apple.terminal"],
        prefixes: &[],
        names: &["terminal"],
    },
    DenyGroup {
        label: "iTerm",
        ids: &["com.googlecode.iterm2"],
        prefixes: &[],
        names: &["iterm", "iterm2"],
    },
    DenyGroup {
        label: "Ghostty",
        ids: &["com.mitchellh.ghostty"],
        prefixes: &[],
        names: &["ghostty"],
    },
    DenyGroup {
        label: "Warp",
        ids: &[],
        prefixes: &["dev.warp."],
        names: &["warp"],
    },
    DenyGroup {
        label: "Alacritty",
        ids: &["org.alacritty"],
        prefixes: &[],
        names: &["alacritty"],
    },
    DenyGroup {
        label: "kitty",
        ids: &["net.kovidgoyal.kitty"],
        prefixes: &[],
        names: &["kitty"],
    },
    DenyGroup {
        label: "WezTerm",
        ids: &["com.github.wez.wezterm"],
        prefixes: &[],
        names: &["wezterm"],
    },
    DenyGroup {
        label: "Script Editor",
        ids: &["com.apple.scripteditor2"],
        prefixes: &[],
        names: &["script editor"],
    },
    DenyGroup {
        label: "System Settings",
        ids: &["com.apple.systempreferences"],
        prefixes: &[],
        names: &["system settings", "system preferences"],
    },
    DenyGroup {
        label: "Keychain Access",
        ids: &["com.apple.keychainaccess"],
        prefixes: &[],
        names: &["keychain access"],
    },
    DenyGroup {
        label: "Passwords",
        ids: &["com.apple.passwords"],
        prefixes: &[],
        names: &["passwords"],
    },
    DenyGroup {
        label: "1Password",
        ids: &[],
        prefixes: &["com.1password.", "com.agilebits."],
        names: &["1password"],
    },
    DenyGroup {
        label: "Bitwarden",
        ids: &["com.bitwarden.desktop"],
        prefixes: &[],
        names: &["bitwarden"],
    },
    DenyGroup {
        label: "LastPass",
        ids: &["com.lastpass.lastpass"],
        prefixes: &[],
        names: &["lastpass"],
    },
    DenyGroup {
        label: "Dashlane",
        ids: &["com.dashlane.dashlanephonefinal"],
        prefixes: &[],
        names: &["dashlane"],
    },
    DenyGroup {
        label: "KeePassXC",
        ids: &["org.keepassxc.keepassxc"],
        prefixes: &[],
        names: &["keepassxc"],
    },
    DenyGroup {
        label: "Cua Driver",
        ids: &[],
        prefixes: &["com.trycua."],
        names: &["cuadriver", "cua driver"],
    },
    DenyGroup {
        label: "Hollow",
        ids: &["com.hollow.app"],
        prefixes: &[],
        names: &["hollow"],
    },
];

impl DenyGroup {
    /// Whether `app` (a lowercase bundle id, or `name:<normalized>`) is this
    /// group: an id or one of its sub bundles, a family prefix, or a name or
    /// a name that starts a new word ("1Password 8", "Hollow Dev").
    fn matches(&self, app: &str) -> bool {
        // A value is tested as an id and as a name whichever it was given as:
        // the Driver may resolve `app: "com.apple.Terminal"` by bundle id.
        let (id, name) = forms(app);
        self.ids.iter().any(|d| sub_bundle(&id, d))
            || self.prefixes.iter().any(|p| id.starts_with(p))
            || self.names.iter().any(|n| word_match(&name, n))
    }

    /// The entries the person's file holds when they allow this group.
    fn allow_entries(&self) -> Vec<String> {
        let mut out: Vec<String> = self.ids.iter().map(|s| s.to_string()).collect();
        out.extend(self.prefixes.iter().map(|p| format!("{p}*")));
        out.extend(self.names.iter().map(|n| format!("name:{n}")));
        out
    }
}

/// A subject as an id and as a name: `name:<n>` gives `n` for both; a bundle
/// id gives itself and its normalized spelling.
fn forms(app: &str) -> (String, String) {
    match app.strip_prefix("name:") {
        Some(n) => (n.to_string(), n.to_string()),
        None => (app.to_string(), normalize_name(app)),
    }
}

fn sub_bundle(app: &str, id: &str) -> bool {
    app == id || app.strip_prefix(id).is_some_and(|r| r.starts_with('.'))
}

fn word_match(name: &str, base: &str) -> bool {
    name == base || name.strip_prefix(base).is_some_and(|r| r.starts_with(' '))
}

/// What a call (or one `run_steps` step) says about where it acts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Look {
    /// A named app: its lowercase bundle id, or `name:<normalized name>` when
    /// only a name was given.
    App(String),
    /// Names no app: a view of the whole display or of all windows.
    Unnamed,
    /// Refused, never carried to a card or the manifest: a name that is not
    /// a plain identifier, a non-string value, a bundle id and a name
    /// together, two different values, or another way to aim at a process.
    Bad,
}

fn plain(s: &str) -> bool {
    !s.is_empty()
        && s.chars().count() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
}

/// An app's display name as compared: lowercase, no `.app`, single spaces.
pub(crate) fn normalize_name(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    let lower = lower.strip_suffix(".app").unwrap_or(&lower);
    lower.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every value a call gives under `keys`: `Err` if any is not a string.
fn values(args: &Value, keys: &[&str]) -> Result<Vec<String>, ()> {
    let mut out = Vec::new();
    for k in keys {
        match args.get(*k) {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if s.trim().is_empty() => {}
            Some(Value::String(s)) => out.push(s.trim().to_string()),
            Some(_) => return Err(()),
        }
    }
    Ok(out)
}

/// The app one call names. Exactly one app, by bundle id or by name, however
/// many keys repeat it; anything else is [`Look::Bad`], so the app that was
/// checked is the only one the Driver is told about.
pub(crate) fn look(args: &Value) -> Look {
    if TARGET_KEYS
        .iter()
        .any(|k| args.get(*k).is_some_and(|v| !v.is_null()))
    {
        return Look::Bad;
    }
    if let Some(map) = args.as_object() {
        for (key, value) in map {
            // Only the recognized keys may name an app; any other key that
            // looks like it aims the call (an alias the Driver might honour)
            // is refused, and so is a nested object (a hidden target).
            let loose = key.to_lowercase().replace(['_', '-'], "");
            let recognized =
                APP_ID_KEYS.contains(&key.as_str()) || APP_NAME_KEYS.contains(&key.as_str());
            if !recognized && !value.is_null() && AIM_WORDS.iter().any(|w| loose.contains(w)) {
                return Look::Bad;
            }
            if value.is_object() {
                return Look::Bad;
            }
        }
    }
    let (Ok(ids), Ok(names)) = (values(args, APP_ID_KEYS), values(args, APP_NAME_KEYS)) else {
        return Look::Bad;
    };
    match (ids.is_empty(), names.is_empty()) {
        (true, true) => Look::Unnamed,
        (false, false) => Look::Bad,
        (false, true) => {
            let norm: BTreeSet<String> = ids.iter().map(|v| v.to_lowercase()).collect();
            match norm.iter().next() {
                Some(id) if norm.len() == 1 && ids.iter().all(|v| plain(v)) => {
                    Look::App(id.clone())
                }
                _ => Look::Bad,
            }
        }
        (true, false) => {
            let norm: BTreeSet<String> = names.iter().map(|v| normalize_name(v)).collect();
            match norm.iter().next() {
                Some(n) if norm.len() == 1 && names.iter().all(|v| plain(v)) => {
                    Look::App(format!("name:{n}"))
                }
                _ => Look::Bad,
            }
        }
    }
}

/// Every look in a call: a `run_steps` call is judged by each step it
/// carries (already rebuilt from checked fields), so a denied app can't hide
/// inside one.
pub(crate) fn looks_for_call(tool: &str, args: &Value) -> Vec<Look> {
    if tool != "run_steps" {
        return vec![look(args)];
    }
    args.get("steps")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .map(|s| look(s.get("arguments").unwrap_or(&Value::Null)))
                .collect()
        })
        .unwrap_or_default()
}

/// The person's allowances over the deny list.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Policy {
    /// Default-denied apps the person chose to allow (lowercase ids or
    /// `name:` entries).
    pub allowed: BTreeSet<String>,
}

impl Policy {
    pub(crate) fn load_from(dir: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(dir.join(POLICY_FILE)) else {
            return Self::default();
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            // An unreadable file never widens anything.
            return Self::default();
        };
        let allowed = v
            .get("allowDenied")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|s| match s.trim().strip_prefix("name:") {
                        Some(n) => format!("name:{}", normalize_name(n)),
                        None => s.trim().to_lowercase(),
                    })
                    .filter(|s| plain(s.trim_start_matches("name:").trim_end_matches('*')))
                    .filter(|s| !s.ends_with('*') || s.ends_with(".*"))
                    .collect()
            })
            .unwrap_or_default();
        Self { allowed }
    }

    pub(crate) fn load() -> Self {
        crate::instance::support_dir()
            .map(|d| Self::load_from(&d))
            .unwrap_or_default()
    }

    /// Whether this app is denied: in a default-denied group the person has
    /// not allowed. An allowance is the group's own entries (an id and its
    /// sub bundles, `prefix*`, a name and names that continue it).
    pub(crate) fn denied(&self, app: &str) -> bool {
        if self.allows(app) {
            return false;
        }
        DENY_GROUPS.iter().any(|g| g.matches(app))
    }

    fn allows(&self, app: &str) -> bool {
        let (id, name) = forms(app);
        self.allowed.iter().any(|entry| {
            if let Some(n) = entry.strip_prefix("name:") {
                return word_match(&name, n) || sub_bundle(&id, n);
            }
            match entry.strip_suffix('*') {
                // Only a whole family (`com.vendor.*`), never a loose prefix.
                Some(prefix) => prefix.ends_with('.') && id.starts_with(prefix),
                None => sub_bundle(&id, entry) || word_match(&name, entry),
            }
        })
    }

    /// Settings' list: each denied group and whether it is allowed now.
    pub(crate) fn groups(&self) -> Vec<(&'static str, bool)> {
        DENY_GROUPS
            .iter()
            .map(|g| {
                let allowed = g.allow_entries().iter().all(|e| self.allowed.contains(e));
                (g.label, allowed)
            })
            .collect()
    }

    /// Allow or block one group by its label. `false` for an unknown label.
    pub(crate) fn set_group(&mut self, label: &str, allow: bool) -> bool {
        let Some(g) = DENY_GROUPS.iter().find(|g| g.label == label) else {
            return false;
        };
        for e in g.allow_entries() {
            if allow {
                self.allowed.insert(e);
            } else {
                self.allowed.remove(&e);
            }
        }
        true
    }

    /// Write the person's allowances (0600, replaced whole). Atomic: a
    /// reader sees the old file or the new one.
    pub(crate) fn save_to(&self, dir: &Path) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot save: {e}"))?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = dir.join(format!(
            "{POLICY_FILE}.tmp-{}-{nanos:x}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&tmp);
        let body = json!({ "allowDenied": self.allowed.iter().collect::<Vec<_>>() });
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| format!("cannot save: {e}"))?;
        f.write_all(body.to_string().as_bytes())
            .map_err(|e| format!("cannot save: {e}"))?;
        f.sync_all().map_err(|e| format!("cannot save: {e}"))?;
        drop(f);
        std::fs::rename(&tmp, dir.join(POLICY_FILE)).map_err(|e| format!("cannot save: {e}"))
    }
}

/// What one run's Driver may reach: the apps and display the run has passed
/// the checks for. It grows with each grant and drops an app the person has
/// since blocked (the Driver restarts narrower).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Manifest {
    pub run: String,
    pub apps: BTreeSet<String>,
    /// A look at the whole display, not one app.
    pub display: bool,
}

impl Manifest {
    pub(crate) fn new(run: &str) -> Self {
        Self {
            run: run.trim().to_string(),
            ..Self::default()
        }
    }

    /// The Driver's `bounded` manifest. Field names unverified (see the
    /// module header); the time bounds are the broker's own.
    pub(crate) fn to_json(&self) -> Value {
        let mut v = crate::computer_broker::run_manifest(&self.run);
        v["apps"] = json!(self
            .apps
            .iter()
            .filter(|a| !a.starts_with("name:"))
            .collect::<Vec<_>>());
        v["app_names"] = json!(self
            .apps
            .iter()
            .filter_map(|a| a.strip_prefix("name:"))
            .collect::<Vec<_>>());
        v["desktop"] = json!({ "display": self.display });
        v
    }

    /// A manifest that allows nothing: for the handshake that learns the tool
    /// list before anyone has been asked.
    pub(crate) fn empty(run: &str) -> Self {
        Self::new(run)
    }
}

/// A manifest file the Driver reads, removed when dropped (the session that
/// owns it ends). 0600 in Hollow's cache folder.
pub(crate) struct ManifestFile {
    path: PathBuf,
}

impl ManifestFile {
    pub(crate) fn write(dir: &Path, manifest: &Manifest) -> Result<Self, String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot write the run's manifest: {e}"))?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = dir.join(format!(
            "computer-manifest-{}-{nanos:x}.json",
            std::process::id()
        ));
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("cannot write the run's manifest: {e}"))?;
        f.write_all(manifest.to_json().to_string().as_bytes())
            .map_err(|e| format!("cannot write the run's manifest: {e}"))?;
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ManifestFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
#[path = "computer_thismac_tests.rs"]
mod tests;
