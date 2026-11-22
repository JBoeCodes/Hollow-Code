//! Which Hollow this process is: the everyday app (`com.hollow.app`) or a
//! side-by-side **candidate** (`com.hollow.app.<label>`, merged plan step 0),
//! read once from the running bundle's `Info.plist`.
//!
//! macOS already keys a bundle's WebKit storage, preferences, notifications
//! and privacy grants by its identifier. Hollow's own stores outside the vault
//! did not follow it: they were spelled `Hollow`, `hollow`, `com.hollow.app`
//! and `com.hollow.secrets` at each call site, so a review build with a new
//! identifier still shared the everyday app's session store, broker sockets,
//! Keychain item, worktrees and config. Every such name now comes from here:
//!
//! | Store | Everyday | Candidate `<label>` |
//! |---|---|---|
//! | app data (`agent_settings::app_data_dir`) | `~/Library/Application Support/com.hollow.app` | `…/com.hollow.app.<label>` |
//! | support folder (Sessions, indexes, leases, permits, heartbeat, config, themes) | `…/Application Support/Hollow` | `…/Application Support/Hollow-<label>` |
//! | caches (broker sockets, build targets) and `~/.cache` | `hollow` | `hollow-<label>` |
//! | logs | `~/Library/Logs/Hollow` | `~/Library/Logs/Hollow-<label>` |
//! | Keychain service | `com.hollow.secrets` | `com.hollow.secrets.<label>`, no legacy import |
//! | worktree root default | `~/Hollow/Worktrees` | `~/Hollow-<label>/Worktrees` |
//!
//! A candidate also reads config only from `$HOLLOW_CONFIG` or its own support
//! folder ([`crate::config`]), refuses a vault or projects folder that overlaps
//! the everyday vault ([`refuse_everyday_vault`]), never installs itself over
//! `/Applications/Hollow.app` ([`crate::self_install`]), skips the global
//! shortcuts and login item, and starts no background agent on its own
//! ([`background_agents_allowed`]). A process outside a bundle (`tauri dev`,
//! `cargo run`, tests) is the everyday instance, as before.
//!
//! This separates Hollow's state, not the user account: agent CLIs and their
//! sign-ins, `~/.claude/skills`, Claude Code's claude.ai connectors and every
//! file an agent's shell can reach are still shared. See docs/history/TEST_BED.md.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const EVERYDAY_ID: &str = "com.hollow.app";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    /// The bundle identifier this process runs under.
    pub identifier: String,
    /// `None` for the everyday app; a filesystem-safe label otherwise.
    pub label: Option<String>,
}

impl Instance {
    pub fn everyday() -> Self {
        Instance {
            identifier: EVERYDAY_ID.into(),
            label: None,
        }
    }

    /// `com.hollow.app` is the everyday app; `com.hollow.app.<label>` is a
    /// candidate called `<label>`; any other identifier is a candidate named
    /// after the whole identifier. Labels keep `[a-z0-9-]` only.
    pub fn from_identifier(identifier: &str) -> Self {
        let identifier = identifier.trim();
        if identifier == EVERYDAY_ID || identifier.is_empty() {
            return Instance::everyday();
        }
        let raw = identifier
            .strip_prefix(EVERYDAY_ID)
            .and_then(|rest| rest.strip_prefix('.'))
            .unwrap_or(identifier);
        let mut label: String = raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        label = label.trim_matches('-').chars().take(48).collect();
        if label.is_empty() {
            label = "candidate".into();
        }
        Instance {
            identifier: identifier.into(),
            label: Some(label),
        }
    }

    pub fn is_candidate(&self) -> bool {
        self.label.is_some()
    }

    fn suffixed(&self, base: &str, sep: char) -> String {
        match &self.label {
            None => base.into(),
            Some(label) => format!("{base}{sep}{label}"),
        }
    }

    /// Folder name under Application Support, Logs and the config dir.
    pub fn support_name(&self) -> String {
        self.suffixed("Hollow", '-')
    }

    /// Folder name under `~/Library/Caches` and `~/.cache`.
    pub fn cache_name(&self) -> String {
        self.suffixed("hollow", '-')
    }

    /// The app-data folder name: the identifier itself.
    pub fn app_data_name(&self) -> String {
        self.identifier.clone()
    }

    /// The Keychain service holding the secret bundle.
    pub fn secret_service(&self) -> String {
        self.suffixed(crate::secret_store::SERVICE, '.')
    }

    /// Whether secrets may be imported from the pre-bundle Keychain items.
    /// Those belong to the everyday app.
    pub fn imports_legacy_secrets(&self) -> bool {
        !self.is_candidate()
    }

    /// The folder `~/<this>/Worktrees` defaults to.
    pub fn home_name(&self) -> String {
        self.support_name()
    }
}

static CURRENT: OnceLock<Instance> = OnceLock::new();

/// This process's instance. Tests are always the everyday instance.
pub fn current() -> &'static Instance {
    CURRENT.get_or_init(|| {
        #[cfg(test)]
        {
            Instance::everyday()
        }
        #[cfg(not(test))]
        {
            std::env::current_exe()
                .ok()
                .and_then(|exe| bundle_identifier(&exe))
                .map(|id| Instance::from_identifier(&id))
                .unwrap_or_else(Instance::everyday)
        }
    })
}

/// `<X>.app/Contents/MacOS/<exe>` → `CFBundleIdentifier` of `<X>.app`.
pub fn bundle_identifier(exe: &Path) -> Option<String> {
    let exe = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    let macos = exe.parent()?;
    if macos.file_name()? != "MacOS" {
        return None;
    }
    let plist = macos.parent()?.join("Info.plist");
    let value = plist::Value::from_file(plist).ok()?;
    value
        .as_dictionary()?
        .get("CFBundleIdentifier")?
        .as_string()
        .map(str::to_string)
}

/// A review launch: a candidate bundle, or the older `HOLLOW_APP_DATA_DIR`
/// recipe. Neither takes the everyday app's global shortcuts or login item.
pub fn is_review_launch() -> bool {
    review_launch_for(current(), std::env::var_os("HOLLOW_APP_DATA_DIR").is_some())
}

pub fn review_launch_for(instance: &Instance, app_data_override: bool) -> bool {
    instance.is_candidate() || app_data_override
}

/// Whether background agents (the silent heartbeat, "Keep learning") may
/// start runs on their own. Always for the everyday app; a candidate only when
/// launched with `HOLLOW_CANDIDATE_BACKGROUND=1`, so the test bed never spends
/// on the user's accounts unprompted.
pub fn background_agents_allowed() -> bool {
    background_allowed_for(
        current(),
        std::env::var("HOLLOW_CANDIDATE_BACKGROUND").ok().as_deref(),
    )
}

pub fn background_allowed_for(instance: &Instance, opt_in: Option<&str>) -> bool {
    !instance.is_candidate() || opt_in.map(str::trim) == Some("1")
}

/// Whether this launch may change, on its own, what it shares with the
/// everyday app: the agent CLIs' automatic updates (a candidate counting only
/// its own runs could update `claude` mid-turn under the everyday app) and
/// the login item (stale candidate login items kept launching old builds).
/// Never for a review launch (plan completion L5, native plan VN1).
pub fn may_change_shared_setup() -> bool {
    !is_review_launch()
}

/// Ordinary tooltip names its binary; legacy review-copy labels stay separate.
pub fn tray_tooltip(instance: &Instance, identity: &crate::app_menu::BuildIdentity) -> String {
    match &instance.label {
        None => identity.short_text(),
        Some(label) => format!("Hollow review copy ({label})"),
    }
}

/// What a review launch says when asked to update or restore an agent CLI.
pub const REVIEW_AGENT_CLI_REFUSAL: &str =
    "This is a review copy of Hollow: the agent CLIs are shared with the everyday app, so update them there.";

/// What a review launch says when asked to open at login.
pub const REVIEW_LOGIN_ITEM_REFUSAL: &str =
    "This is a review copy of Hollow: only the everyday app can open at login.";

/// Set only by the retired test bed (docs/history/TEST_BED.md); kept pending
/// an owner call. A candidate launched with
/// `HOLLOW_CANDIDATE_AGENT_BIN=<absolute folder>` runs the agent CLIs it
/// finds there in place of the installed ones, so the bed can drive a turn
/// with a fake CLI that prints nothing and bills no account. Agent CLIs
/// resolve from fixed install locations before `$PATH` (a Finder-launched
/// app has the bare system PATH), so a fake placed first on `$PATH` is never
/// reached. The everyday app ignores the variable.
pub const CANDIDATE_AGENT_BIN_ENV: &str = "HOLLOW_CANDIDATE_AGENT_BIN";

/// The fake-agent folder this candidate was launched with, if any.
pub fn candidate_agent_dir() -> Option<PathBuf> {
    candidate_agent_dir_for(
        current(),
        std::env::var_os(CANDIDATE_AGENT_BIN_ENV).as_deref(),
    )
}

/// [`candidate_agent_dir`] for `instance` and the variable's `value`: only a
/// candidate, and only an absolute folder.
pub fn candidate_agent_dir_for(
    instance: &Instance,
    value: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    if !instance.is_candidate() {
        return None;
    }
    let dir = PathBuf::from(value?);
    dir.is_absolute().then_some(dir)
}

/// The agent CLI `name` (`claude`, `codex`, `agy`, `grok`) in
/// [`candidate_agent_dir`], when that file exists. Both resolvers
/// (`agent::find_binary`, `chat::find_claude_binary`) ask this first.
pub fn candidate_agent_bin(name: &str) -> Option<PathBuf> {
    candidate_agent_bin_in(&candidate_agent_dir()?, name)
}

fn candidate_agent_bin_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let bin = dir.join(name);
    bin.is_file().then_some(bin)
}

/// Set only by the retired test bed (docs/history/TEST_BED.md); kept pending
/// an owner call. A candidate launched with
/// `HOLLOW_CANDIDATE_JEV_ENDPOINT=http://127.0.0.1:<port>/…` sends every
/// Jev request (`jev::endpoint`) to that loopback fake instead of TypeSafe,
/// so a bed row can switch Jev on with a dummy key and never reach the
/// network. Only a plain-http URL to a literal loopback IP is honoured
/// (not `localhost`, a name the system resolver answers); the everyday app
/// ignores the variable.
pub const CANDIDATE_JEV_ENDPOINT_ENV: &str = "HOLLOW_CANDIDATE_JEV_ENDPOINT";

/// The loopback Jev endpoint this candidate was launched with, if any.
pub fn candidate_jev_endpoint() -> Option<String> {
    candidate_jev_endpoint_for(
        current(),
        std::env::var_os(CANDIDATE_JEV_ENDPOINT_ENV).as_deref(),
    )
}

/// [`candidate_jev_endpoint`] for `instance` and the variable's `value`:
/// only a candidate, and only `http://` to the literal loopback IP
/// 127.0.0.1 or `[::1]` (never a host name, `localhost` included: what it
/// resolves to is the resolver's call), with no credentials in the URL.
pub fn candidate_jev_endpoint_for(
    instance: &Instance,
    value: Option<&std::ffi::OsStr>,
) -> Option<String> {
    if !instance.is_candidate() {
        return None;
    }
    loopback_jev_url(value?.to_str()?)
}

/// `raw` as a loopback Jev endpoint: `http://` to the literal IP 127.0.0.1
/// or `[::1]` (never a host name, `localhost` included), no credentials.
/// The one rule for the bed candidate's fake ([`candidate_jev_endpoint_for`])
/// and `hollow jev-study --endpoint` (`jev_study`).
pub fn loopback_jev_url(raw: &str) -> Option<String> {
    let url = reqwest::Url::parse(raw.trim()).ok()?;
    // The parser writes an IP host in its canonical form (`127.1` is
    // `127.0.0.1`, `[0:0::1]` is `[::1]`), so this compares IPs, not names.
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]"));
    (url.scheme() == "http" && loopback && url.username().is_empty() && url.password().is_none())
        .then(|| url.to_string())
}

// ── Roots ────────────────────────────────────────────────────────────────

/// `hollow serve --data-dir <dir>`: the support and cache roots live under
/// `<dir>` too, so a serve beside this Mac's own Hollow shares none of its
/// stores (M1 PR 2 review). Never set in the app.
static ISOLATED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Move [`support_dir`] to `<dir>/Support` and the cache roots under
/// `<dir>/Caches` for the rest of the process (`hollow serve --data-dir`),
/// before any store opens.
pub(crate) fn isolate_under(dir: &Path) {
    let _ = ISOLATED.set(dir.to_path_buf());
}

/// `~/Library/Application Support/<Hollow|Hollow-label>` (`dirs::data_dir`),
/// or `<dir>/Support` under `hollow serve --data-dir` ([`isolate_under`]).
pub fn support_dir() -> Option<PathBuf> {
    if let Some(dir) = ISOLATED.get() {
        return Some(dir.join("Support"));
    }
    default_support_dir()
}

/// The support folder the app uses, whatever [`isolate_under`] said.
pub(crate) fn default_support_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(current().support_name()))
}

/// The root cache folders live in: `~/Library/Caches`, else the temp dir.
fn cache_root() -> PathBuf {
    if let Some(dir) = ISOLATED.get() {
        return dir.join("Caches");
    }
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir)
}

/// `~/Library/Caches/<hollow|hollow-label>`, else the temp dir.
pub fn cache_dir() -> PathBuf {
    cache_root().join(current().cache_name())
}

/// A sibling cache folder such as `hollow-build-targets`, suffixed per instance.
pub fn cache_sibling(stem: &str) -> PathBuf {
    let name = match &current().label {
        None => format!("hollow-{stem}"),
        Some(label) => format!("hollow-{label}-{stem}"),
    };
    cache_root().join(name)
}

/// `~/.cache/<hollow|hollow-label>`.
pub fn dot_cache_dir(home: &Path) -> PathBuf {
    home.join(".cache").join(current().cache_name())
}

/// `~/Library/Logs/<Hollow|Hollow-label>`.
pub fn logs_dir(home: &Path) -> PathBuf {
    home.join("Library/Logs").join(current().support_name())
}

// ── The everyday vault ───────────────────────────────────────────────────

/// The vault the everyday app is configured with, read from the config files
/// it looks in (not `$HOLLOW_CONFIG`, which belongs to this launch).
pub fn everyday_vault(home: &Path) -> Option<PathBuf> {
    let candidates = [
        home.join(".config/Hollow/config.json"),
        home.join("Library/Application Support/Hollow/config.json"),
    ];
    candidates.iter().find_map(|path| {
        let text = std::fs::read_to_string(path).ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        let vault = value.get("vaultPath")?.as_str()?;
        Some(crate::setup::expand_tilde(vault))
    })
}

fn normal(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Two folders overlap when one is, or sits inside, the other. Compared
/// case-insensitively, as APFS is.
pub fn overlaps(a: &Path, b: &Path) -> bool {
    let a = normal(a).to_string_lossy().to_lowercase();
    let b = normal(b).to_string_lossy().to_lowercase();
    let a = a.trim_end_matches('/');
    let b = b.trim_end_matches('/');
    a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
}

/// A candidate must never open the everyday vault. `None` when allowed.
pub fn refuse_everyday_vault_for(
    instance: &Instance,
    home: &Path,
    folders: &[&Path],
) -> Option<String> {
    if !instance.is_candidate() {
        return None;
    }
    let everyday = everyday_vault(home)?;
    folders.iter().find(|folder| overlaps(folder, &everyday)).map(|folder| {
        format!(
            "This is a candidate build ({}). It will not open {} because it overlaps the everyday Hollow vault {}. Point HOLLOW_CONFIG at a synthetic vault.",
            instance.identifier,
            folder.display(),
            everyday.display()
        )
    })
}

pub fn refuse_everyday_vault(folders: &[&Path]) -> Option<String> {
    refuse_everyday_vault_for(current(), &dirs::home_dir()?, folders)
}

// ── What this launch uses ────────────────────────────────────────────────

/// `{app data}/instance.json`: written at GUI start so the test bed can check
/// what a running build is, and which folders it resolved, without asking it.
pub fn write_manifest(config_path: &Path, vault: Option<&Path>) {
    let Some(value) = manifest(config_path, vault) else {
        return;
    };
    let Ok(dir) = crate::agent_settings::app_data_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("instance.json");
    let tmp = dir.join("instance.json.tmp");
    let text = serde_json::to_string_pretty(&value).unwrap_or_default() + "\n";
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn manifest(config_path: &Path, vault: Option<&Path>) -> Option<serde_json::Value> {
    let instance = current();
    let home = dirs::home_dir()?;
    let build = crate::app_menu::build_identity();
    let path = |p: Option<PathBuf>| p.map(|p| p.display().to_string());
    Some(serde_json::json!({
        "identifier": instance.identifier,
        "label": instance.label,
        "pid": std::process::id(),
        "exe": path(std::env::current_exe().ok()),
        "commit": build.commit,
        "dirty": build.dirty,
        "sourceFingerprint": build.source_fingerprint,
        "configPath": config_path.display().to_string(),
        "vaultPath": path(vault.map(Path::to_path_buf)),
        "appDataDir": path(crate::agent_settings::app_data_dir().ok()),
        "supportDir": path(support_dir()),
        "cacheDir": cache_dir().display().to_string(),
        "dotCacheDir": dot_cache_dir(&home).display().to_string(),
        "logsDir": logs_dir(&home).display().to_string(),
        "secretService": instance.secret_service(),
        "worktreesDefault": home.join(instance.home_name()).join("Worktrees").display().to_string(),
        "agentBin": path(candidate_agent_dir()),
        "jevEndpoint": candidate_jev_endpoint(),
        "startedAt": chrono::Utc::now().to_rfc3339(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_everyday_identifier_keeps_every_existing_name() {
        let i = Instance::from_identifier(EVERYDAY_ID);
        assert!(!i.is_candidate());
        assert_eq!(i.support_name(), "Hollow");
        assert_eq!(i.cache_name(), "hollow");
        assert_eq!(i.app_data_name(), "com.hollow.app");
        assert_eq!(i.secret_service(), "com.hollow.secrets");
        assert!(i.imports_legacy_secrets());
        assert_eq!(
            current(),
            &Instance::everyday(),
            "tests run as the everyday app"
        );
    }

    #[test]
    fn a_candidate_identifier_renames_every_store() {
        let c = Instance::from_identifier("com.hollow.app.candidate");
        let e = Instance::everyday();
        assert_eq!(c.label.as_deref(), Some("candidate"));
        assert_eq!(c.support_name(), "Hollow-candidate");
        assert_eq!(c.cache_name(), "hollow-candidate");
        assert_eq!(c.app_data_name(), "com.hollow.app.candidate");
        assert_eq!(c.secret_service(), "com.hollow.secrets.candidate");
        assert!(
            !c.imports_legacy_secrets(),
            "legacy Keychain items are the everyday app's"
        );
        for (a, b) in [
            (c.support_name(), e.support_name()),
            (c.cache_name(), e.cache_name()),
            (c.app_data_name(), e.app_data_name()),
            (c.secret_service(), e.secret_service()),
            (c.home_name(), e.home_name()),
        ] {
            assert_ne!(
                a.to_lowercase(),
                b.to_lowercase(),
                "APFS is case-insensitive"
            );
        }
    }

    #[test]
    fn a_candidate_runs_no_background_agent_unless_asked() {
        let candidate = Instance::from_identifier("com.hollow.app.candidate");
        assert!(!background_allowed_for(&candidate, None));
        assert!(!background_allowed_for(&candidate, Some("true")));
        assert!(background_allowed_for(&candidate, Some("1")));
        assert!(
            background_allowed_for(&Instance::everyday(), None),
            "the everyday app is unchanged"
        );
    }

    #[test]
    fn odd_identifiers_become_safe_labels() {
        assert_eq!(
            Instance::from_identifier("com.hollow.app.Studio Review")
                .label
                .as_deref(),
            Some("studio-review")
        );
        assert_eq!(
            Instance::from_identifier("com.example.other")
                .label
                .as_deref(),
            Some("com-example-other")
        );
        assert_eq!(
            Instance::from_identifier("com.hollow.app.../")
                .label
                .as_deref(),
            Some("candidate")
        );
        assert_eq!(Instance::from_identifier("  "), Instance::everyday());
    }

    #[test]
    fn the_bundle_identifier_is_read_from_the_enclosing_app() {
        let dir = tempfile::tempdir().unwrap();
        let macos = dir.path().join("Hollow Candidate.app/Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let mut dict = plist::Dictionary::new();
        dict.insert(
            "CFBundleIdentifier".into(),
            plist::Value::String("com.hollow.app.candidate".into()),
        );
        plist::Value::Dictionary(dict)
            .to_file_xml(macos.parent().unwrap().join("Info.plist"))
            .unwrap();
        let exe = macos.join("hollow");
        std::fs::write(&exe, b"").unwrap();
        assert_eq!(
            bundle_identifier(&exe).as_deref(),
            Some("com.hollow.app.candidate")
        );
        // Outside a bundle (cargo run, tauri dev): no identifier, so everyday.
        let loose = dir.path().join("hollow");
        std::fs::write(&loose, b"").unwrap();
        assert_eq!(bundle_identifier(&loose), None);
    }

    fn home_with_everyday_vault(vault: &Path) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let cfg = home.path().join("Library/Application Support/Hollow");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(
            cfg.join("config.json"),
            serde_json::json!({ "vaultPath": vault }).to_string(),
        )
        .unwrap();
        home
    }

    #[test]
    fn a_candidate_refuses_the_everyday_vault_and_anything_overlapping_it() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("hollow-vault");
        let synthetic = root.path().join("candidate-vault");
        std::fs::create_dir_all(real.join("workspaces")).unwrap();
        std::fs::create_dir_all(&synthetic).unwrap();
        let home = home_with_everyday_vault(&real);
        let candidate = Instance::from_identifier("com.hollow.app.candidate");

        assert!(refuse_everyday_vault_for(&candidate, home.path(), &[&synthetic]).is_none());
        for bad in [
            real.clone(),
            real.join("workspaces"),
            root.path().to_path_buf(),
            PathBuf::from(real.to_string_lossy().to_uppercase()),
            real.join("workspaces/.."),
        ] {
            let refusal = refuse_everyday_vault_for(&candidate, home.path(), &[&synthetic, &bad]);
            assert!(refusal.is_some(), "{bad:?} overlaps the everyday vault");
        }
        // The everyday app is never refused its own vault.
        assert!(refuse_everyday_vault_for(&Instance::everyday(), home.path(), &[&real]).is_none());
    }

    #[test]
    fn overlap_is_by_whole_path_components() {
        assert!(!overlaps(Path::new("/x/vault-two"), Path::new("/x/vault")));
        assert!(overlaps(Path::new("/x/vault/"), Path::new("/x/vault")));
    }

    #[test]
    fn only_a_candidate_takes_the_fake_agent_folder() {
        let candidate = Instance::from_identifier("com.hollow.app.candidate");
        let folder = std::ffi::OsStr::new("/bed/fake-agent/bin");
        assert_eq!(
            candidate_agent_dir_for(&candidate, Some(folder)),
            Some(PathBuf::from("/bed/fake-agent/bin"))
        );
        assert_eq!(
            candidate_agent_dir_for(&Instance::everyday(), Some(folder)),
            None,
            "the everyday app always runs the installed agents"
        );
        assert_eq!(candidate_agent_dir_for(&candidate, None), None);
        for odd in ["", "fake-agent/bin"] {
            assert_eq!(
                candidate_agent_dir_for(&candidate, Some(std::ffi::OsStr::new(odd))),
                None,
                "{odd:?}: only an absolute folder"
            );
        }
        assert_eq!(candidate_agent_dir(), None, "tests are the everyday app");

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(candidate_agent_bin_in(dir.path(), "claude"), None);
        std::fs::write(dir.path().join("claude"), "#!/bin/sh\n").unwrap();
        assert_eq!(
            candidate_agent_bin_in(dir.path(), "claude"),
            Some(dir.path().join("claude"))
        );
        assert_eq!(candidate_agent_bin_in(dir.path(), "codex"), None);
    }

    #[test]
    fn only_a_candidate_honours_a_loopback_jev_endpoint() {
        let candidate = Instance::from_identifier("com.hollow.app.candidate");
        let fake = std::ffi::OsStr::new("http://127.0.0.1:47613/v1/systemone");
        assert_eq!(
            candidate_jev_endpoint_for(&candidate, Some(fake)).as_deref(),
            Some("http://127.0.0.1:47613/v1/systemone")
        );
        assert_eq!(
            candidate_jev_endpoint_for(&Instance::everyday(), Some(fake)),
            None,
            "the everyday app always uses TypeSafe's endpoint"
        );
        assert_eq!(
            candidate_jev_endpoint_for(&Instance::from_identifier(EVERYDAY_ID), Some(fake)),
            None
        );
        for ok in [
            "http://[::1]:9/x",
            "http://127.0.0.1/x",
            "http://[0:0:0:0:0:0:0:1]:9/x",
        ] {
            assert!(
                candidate_jev_endpoint_for(&candidate, Some(std::ffi::OsStr::new(ok))).is_some(),
                "{ok}"
            );
        }
        for refused in [
            "",
            "https://api.typesafe.ai/v1/systemone",
            "http://api.typesafe.ai/v1/systemone",
            "http://10.0.0.5:9/x",
            "http://127.0.0.1.example.com/x",
            "https://127.0.0.1:9/x",
            "http://user:pw@127.0.0.1:9/x",
            "127.0.0.1:9",
            // A name, not a literal loopback IP: resolution is up to the
            // system's resolver (wave 1 follow-up to R2).
            "http://localhost:9/x",
            "http://LOCALHOST:9/x",
            "http://localhost.:9/x",
        ] {
            assert_eq!(
                candidate_jev_endpoint_for(&candidate, Some(std::ffi::OsStr::new(refused))),
                None,
                "{refused:?}: only a plain-http literal loopback IP"
            );
        }
        assert_eq!(candidate_jev_endpoint_for(&candidate, None), None);
        assert_eq!(candidate_jev_endpoint(), None, "tests are the everyday app");
        assert_eq!(
            crate::jev::endpoint(),
            "https://api.typesafe.ai/v1/systemone"
        );
    }

    #[test]
    fn every_jev_request_goes_through_the_one_endpoint() {
        // The override can only work (and the everyday app can only ignore
        // it) if no caller names TypeSafe's URL itself.
        for (file, src) in [
            ("jev_decide.rs", include_str!("jev_decide.rs")),
            ("context_rank.rs", include_str!("context_rank.rs")),
        ] {
            assert!(!src.contains("jev::ENDPOINT"), "{file}");
            assert!(!src.contains("api.typesafe.ai"), "{file}");
        }
    }

    #[test]
    fn both_agent_resolvers_ask_for_the_fake_agent_first() {
        use crate::source_pins::find;
        for (file, src, start, ask, installed) in [
            (
                "chat.rs",
                include_str!("chat.rs"),
                "pub fn find_claude_binary() -> Option<PathBuf> {",
                r#"crate::instance::candidate_agent_bin("claude")"#,
                r#"candidates.push(home.join(".local/bin/claude"));"#,
            ),
            (
                "agent.rs",
                include_str!("agent.rs"),
                "fn concrete_binary(kind: AgentKind) -> Option<PathBuf> {",
                "crate::instance::candidate_agent_bin(bin_name(kind))",
                "candidates(kind).into_iter().find(|p| p.exists())",
            ),
        ] {
            let at = find(src, start).unwrap_or_else(|| panic!("{file}: {start}"));
            let body = &src[at..];
            let ask_at = find(body, ask).unwrap_or_else(|| panic!("{file}: {ask}"));
            let inst = find(body, installed).unwrap_or_else(|| panic!("{file}: {installed}"));
            assert!(
                ask_at < inst,
                "{file}: the fake-agent folder comes before the installed locations"
            );
        }
    }
    #[test]
    fn ordinary_tray_names_the_embedded_build() {
        let identity = crate::app_menu::build_identity();
        let tooltip = tray_tooltip(&Instance::from_identifier(EVERYDAY_ID), &identity);
        assert!(tooltip.contains(&identity.version));
        assert!(tooltip.contains(&identity.short_commit));
    }

    #[test]
    fn a_review_launch_never_changes_shared_setup() {
        let everyday = Instance::from_identifier(EVERYDAY_ID);
        let candidate = Instance::from_identifier("com.hollow.app.vn1014");
        assert!(!review_launch_for(&everyday, false));
        assert!(review_launch_for(&candidate, false));
        // The older HOLLOW_APP_DATA_DIR recipe is a review launch too.
        assert!(review_launch_for(&everyday, true));
        let identity =
            crate::app_menu::parse_identity(r#"{"version":"0.5.15","commit":"e928b5bd"}"#, "0");
        assert_eq!(
            tray_tooltip(&everyday, &identity),
            "Hollow 0.5.15 · e928b5b"
        );
        assert_eq!(
            tray_tooltip(&candidate, &identity),
            "Hollow review copy (vn1014)"
        );
    }
}
