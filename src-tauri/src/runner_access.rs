//! Remote runners (2026-11-03): what a turn on the runner may reach besides
//! its own checkout. GitHub (push, pull requests) and Google Cloud, chosen per
//! workspace, both off until the person turns them on.
//!
//! - **Where the choice lives:** app data (`runner-access.json`), never the
//!   vault, so an agent that writes the vault can't grant its own turns
//!   credentials. The IPCs here are the person's clicks (Runs on → On the
//!   runner, Settings, Devices, Remote runner); the COO's
//!   `build.set_runner_access` changes it only on its card (COO parity L13).
//! - **What a turn carries:** [`TurnAccess`], resolved when the turn is
//!   prepared and kept with its record. It holds no secret.
//! - **Secrets** are read when the run's leader is built ([`credentials`]):
//!   the GitHub runner token from this Mac's Keychain (or `gh auth token` when
//!   the person picked this Mac's gh login), and the argv that mints a Google
//!   Cloud access token here. They go only into the leader's environment
//!   (`/usr/bin/perl`, whose environment `ps -E` doesn't show), which hands a
//!   credential out one request at a time through the run's own tool socket
//!   (`remote_session::LEADER_PL`, `REMOTE_RUN_PL --credential`/`--wrap`).
//!   Never on a command line, in the run header, in a file on the runner
//!   beyond one gcloud call, in the turn's record, or in a log line.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The Keychain key of the GitHub runner token (Hollow's own secret bundle).
pub(crate) const GITHUB_TOKEN_KEY: &str = "runner.github.token";

/// How a runner turn reaches Google Cloud.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloudAccess {
    /// Not at all (the default).
    #[default]
    Off,
    /// As the person, with production changes refused on the runner.
    Limited,
    /// As the person, everything their account may do.
    Full,
    /// As a service account the person's account may impersonate.
    ServiceAccount,
}

impl CloudAccess {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            CloudAccess::Off => "off",
            CloudAccess::Limited => "limited",
            CloudAccess::Full => "full",
            CloudAccess::ServiceAccount => "service-account",
        }
    }
}

/// One workspace's choice (the UI's shape).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerAccess {
    #[serde(default)]
    pub github: bool,
    #[serde(default)]
    pub cloud: CloudAccess,
    /// The service account to impersonate (`cloud: service-account`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_account: Option<String>,
}

/// Where the GitHub credential comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GithubSource {
    /// A fine-grained token the person made for the runner, in the Keychain.
    #[default]
    RunnerToken,
    /// This Mac's `gh` login (broader: whatever that login may do).
    GhLogin,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store {
    #[serde(default)]
    github_source: GithubSource,
    /// Vault file stem → workspace id → its choice.
    #[serde(default)]
    workspaces: BTreeMap<String, BTreeMap<String, RunnerAccess>>,
}

fn store_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join("runner-access.json"))
}

fn load() -> Result<Store, String> {
    let path = store_path()?;
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn change(f: impl FnOnce(&mut Store) -> Result<(), String>) -> Result<(), String> {
    let _held = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = load()?;
    f(&mut store)?;
    let text = serde_json::to_string_pretty(&store).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(&store_path()?, text.as_bytes())
}

/// Workspace `id`'s choice (all off when it never made one).
pub(crate) fn access_for(vault: &Path, id: &str) -> RunnerAccess {
    load()
        .ok()
        .and_then(|s| {
            s.workspaces
                .get(&crate::session_store::vault_file_stem(vault))?
                .get(id)
                .cloned()
        })
        .unwrap_or_default()
}

/// Whether `email` looks like a Google service account
/// (`name@project.iam.gserviceaccount.com`, or a Google-managed one).
pub(crate) fn service_account_ok(email: &str) -> bool {
    let Some((name, domain)) = email.split_once('@') else {
        return false;
    };
    !name.is_empty()
        && name.len() <= 100
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && domain.ends_with("gserviceaccount.com")
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// Set workspace `id`'s choice; returns what it is now. The Main agent and
/// Quick chats never run on a runner, so they have none.
pub(crate) fn set_access(
    vault: &Path,
    id: &str,
    access: RunnerAccess,
) -> Result<RunnerAccess, String> {
    if crate::remote_folder::never_moves(id) {
        return Err("The COO and Chats always run on this Mac.".into());
    }
    if id.is_empty() || id.contains('/') || id.starts_with('.') {
        return Err(format!("{id} is not a workspace"));
    }
    if !vault.join("workspaces").join(id).is_dir() {
        return Err(format!("{id} is not a workspace"));
    }
    let mut access = access;
    access.service_account = access
        .service_account
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match (access.cloud, access.service_account.as_deref()) {
        (CloudAccess::ServiceAccount, None) => {
            return Err(
                "Name the service account to use (name@project.iam.gserviceaccount.com).".into(),
            )
        }
        (CloudAccess::ServiceAccount, Some(sa)) if !service_account_ok(sa) => {
            return Err(format!(
                "{sa} isn't a service account address (name@project.iam.gserviceaccount.com)."
            ))
        }
        (CloudAccess::ServiceAccount, _) => {}
        // Kept only while it is the one in use.
        _ => access.service_account = None,
    }
    let stem = crate::session_store::vault_file_stem(vault);
    let saved = access.clone();
    change(move |store| {
        let entry = store.workspaces.entry(stem.clone()).or_default();
        if saved == RunnerAccess::default() {
            entry.remove(id);
        } else {
            entry.insert(id.to_string(), saved);
        }
        if entry.is_empty() {
            store.workspaces.remove(&stem);
        }
        Ok(())
    })?;
    Ok(access)
}

/// The workspace whose turn runs in `cwd`: a build workspace's worktree, or a
/// vault workspace's own folder.
pub(crate) fn workspace_for_cwd(vault: &Path, cwd: &Path) -> Option<String> {
    crate::build_workspaces::workspace_for_worktree(vault, cwd)
        .or_else(|| crate::remote_folder::workspace_of(vault, cwd))
}

/// What one runner turn may reach, decided when it is prepared. No secret.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnAccess {
    #[serde(default)]
    pub github: bool,
    #[serde(default)]
    pub cloud: CloudAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_account: Option<String>,
    /// `owner/name` of the checkout's GitHub `origin`, for `gh` there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gh_repo: Option<String>,
}

impl TurnAccess {
    pub(crate) fn any(&self) -> bool {
        self.github || self.cloud != CloudAccess::Off
    }
}

/// The access a turn in `cwd` gets: its workspace's choice, plus the GitHub
/// repository of `checkout`'s `origin` when the turn sends a git checkout (a
/// build workspace's worktree). A vault workspace's folder turn gets none:
/// the folder may sit inside some other repository (the vault's own).
pub(crate) fn for_turn(vault: &Path, cwd: &Path, checkout: Option<&Path>) -> TurnAccess {
    let Some(id) = workspace_for_cwd(vault, cwd) else {
        return TurnAccess::default();
    };
    let access = access_for(vault, &id);
    let gh_repo = match checkout {
        Some(top) if access.github => origin_url(top).and_then(|url| github_repo(&url)),
        _ => None,
    };
    TurnAccess {
        github: access.github,
        cloud: access.cloud,
        service_account: access.service_account,
        gh_repo,
    }
}

fn origin_url(cwd: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-C"])
        .arg(cwd)
        .args(["config", "--get", "remote.origin.url"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `owner/name` of a github.com remote URL (https, scp-like or ssh), else
/// None. Other hosts (GitHub Enterprise, GitLab) aren't handled.
pub(crate) fn github_repo(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com:22/"))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
            && s != "."
            && s != ".."
    };
    (ok(owner) && ok(name)).then(|| format!("{owner}/{name}"))
}

// ---- GitHub credential source (Settings, Devices, Remote runner) ----------

/// What Settings shows: never the token itself.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GithubStatus {
    pub source: GithubSource,
    /// A runner token is saved in this Mac's Keychain.
    pub token_saved: bool,
    /// This Mac's gh is signed in (it answered `gh auth token`).
    pub gh_login: bool,
}

pub(crate) fn github_source() -> GithubSource {
    load().map(|s| s.github_source).unwrap_or_default()
}

pub(crate) fn set_github_source(source: GithubSource) -> Result<(), String> {
    change(|store| {
        store.github_source = source;
        Ok(())
    })
}

pub(crate) fn github_status() -> GithubStatus {
    GithubStatus {
        source: github_source(),
        token_saved: matches!(crate::secret_store::present(GITHUB_TOKEN_KEY), Ok(true)),
        gh_login: gh_login_token().is_ok(),
    }
}

/// Whether `token` looks like a GitHub token: one line, no spaces, a known
/// prefix (`github_pat_` for fine-grained, `ghp_` classic). The token is
/// never echoed in an error.
pub(crate) fn github_token_ok(token: &str) -> Result<(), String> {
    let t = token.trim();
    if t.is_empty() {
        return Err("Paste the token GitHub showed when you made it.".into());
    }
    if t.len() > 255 || !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(
            "That doesn't look like a GitHub token (letters, digits and underscores, one line)."
                .into(),
        );
    }
    if !(t.starts_with("github_pat_") || t.starts_with("ghp_")) {
        return Err("That doesn't look like a GitHub personal access token (it starts with github_pat_ for a fine-grained one).".into());
    }
    Ok(())
}

pub(crate) fn save_github_token(token: &str) -> Result<(), String> {
    github_token_ok(token)?;
    crate::secret_store::set(GITHUB_TOKEN_KEY, token.trim()).map_err(|e| e.to_string())
}

pub(crate) fn clear_github_token() -> Result<(), String> {
    crate::secret_store::delete(GITHUB_TOKEN_KEY).map_err(|e| e.to_string())
}

/// The gh program on this Mac (Hollow opened from Finder has a short PATH).
fn find_program(name: &str, extra: &[&str]) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let home = dirs::home_dir().unwrap_or_default();
    for e in extra {
        dirs.push(match e.strip_prefix("~/") {
            Some(rel) => home.join(rel),
            None => PathBuf::from(e),
        });
    }
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// `gh auth token` on this Mac, bounded to 10 s.
fn gh_login_token() -> Result<String, String> {
    let gh = find_program("gh", &["/opt/homebrew/bin", "/usr/local/bin"])
        .ok_or("gh isn't installed on this Mac")?;
    let out = run_bounded(
        std::process::Command::new(gh).args(["auth", "token", "--hostname", "github.com"]),
        std::time::Duration::from_secs(10),
    )?;
    let token = out.trim().to_string();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err("this Mac's gh isn't signed in to github.com".into());
    }
    Ok(token)
}

/// `gh <args>`'s output on this Mac, bounded to 10 s (SK1's org list).
pub(crate) fn gh_output(args: &[&str]) -> Result<String, String> {
    let gh = find_program("gh", &["/opt/homebrew/bin", "/usr/local/bin"])
        .ok_or("gh isn't installed on this Mac")?;
    run_bounded(
        std::process::Command::new(gh).args(args),
        std::time::Duration::from_secs(10),
    )
}

fn run_bounded(
    cmd: &mut std::process::Command,
    limit: std::time::Duration,
) -> Result<String, String> {
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut so) = child.stdout.take() {
                    use std::io::Read;
                    let _ = so.read_to_string(&mut out);
                }
                return if status.success() {
                    Ok(out)
                } else {
                    Err("this Mac's gh isn't signed in to github.com".into())
                };
            }
            Ok(None) if started.elapsed() < limit => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("gh didn't answer within 10 seconds".into());
            }
        }
    }
}

/// The gcloud program on this Mac.
pub(crate) fn gcloud_program() -> Option<PathBuf> {
    find_program(
        "gcloud",
        &[
            "~/google-cloud-sdk/bin",
            "/opt/homebrew/bin",
            "/opt/homebrew/share/google-cloud-sdk/bin",
            "/usr/local/bin",
        ],
    )
}

// ---- Secrets for one run's leader -------------------------------------------

/// What the run's leader holds. `Err` carries why a credential the
/// workspace turned on isn't available (said by the runner's helper when the
/// agent asks), never a secret.
pub(crate) struct Credentials {
    pub github: Option<Result<String, String>>,
    pub gcloud_mint: Option<Result<Vec<String>, String>>,
    /// The Google Cloud access level the leader returns with each token.
    pub gcloud_level: Option<CloudAccess>,
}

/// Read the secrets `access` asks for. Done when the leader is built (each
/// attempt, a reattach too), so nothing secret is kept with the turn.
pub(crate) fn credentials(access: &TurnAccess) -> Credentials {
    let github = access.github.then(|| match github_source() {
        GithubSource::RunnerToken => match crate::secret_store::get(GITHUB_TOKEN_KEY) {
            Ok(Some(t)) if !t.is_empty() => Ok(t),
            Ok(_) => Err("no runner token is saved on the Mac that started this run (Settings → Devices → Runner GitHub access)".to_string()),
            Err(e) => Err(format!("the runner token couldn't be read from this Mac's Keychain ({e})")),
        },
        GithubSource::GhLogin => gh_login_token(),
    });
    let gcloud_mint = (access.cloud != CloudAccess::Off).then(|| {
        let gcloud = gcloud_program()
            .ok_or_else(|| "gcloud isn't installed on the Mac that started this run".to_string())?;
        let mut argv = vec![
            gcloud.to_string_lossy().into_owned(),
            "auth".to_string(),
            "print-access-token".to_string(),
            "--quiet".to_string(),
        ];
        if access.cloud == CloudAccess::ServiceAccount {
            let sa = access
                .service_account
                .as_deref()
                .filter(|s| service_account_ok(s))
                .ok_or("no valid service account is named for this workspace")?;
            argv.push(format!("--impersonate-service-account={sa}"));
        }
        Ok(argv)
    });
    Credentials {
        github,
        gcloud_mint,
        gcloud_level: (access.cloud != CloudAccess::Off).then_some(access.cloud),
    }
}

/// The run header's additions for `access` (flags only, no secret). With
/// GitHub on, the runner's supervisor writes the run's own git configuration
/// from them (`REMOTE_RUN_PL`'s `run_git_config`): a credential helper for
/// github.com, https for GitHub's ssh URLs, and `origin` only inside the
/// run's own checkout. A file, not `GIT_CONFIG_KEY_n`, because Codex drops a
/// variable whose name holds KEY from its commands, and environment config
/// would reach every repository the agent touches.
pub(crate) fn header_env(access: &TurnAccess, env: &mut BTreeMap<String, String>) {
    if access.github {
        env.insert("HOLLOW_RUNNER_GITHUB".into(), "on".into());
        // Never a prompt with no one to answer it.
        env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        env.insert("GH_PROMPT_DISABLED".into(), "1".into());
        if let Some(repo) = &access.gh_repo {
            env.insert("GH_REPO".into(), repo.clone());
            env.insert(
                "HOLLOW_RUNNER_ORIGIN".into(),
                format!("https://github.com/{repo}.git"),
            );
        }
    }
    if access.cloud != CloudAccess::Off {
        env.insert("HOLLOW_RUNNER_CLOUD".into(), access.cloud.as_str().into());
    }
}

/// The leader's environment for `creds` (secrets: only ever the leader's).
pub(crate) fn leader_env(creds: &Credentials) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    match &creds.github {
        Some(Ok(token)) => out.push(("HOLLOW_CRED_GITHUB", token.clone())),
        Some(Err(why)) => out.push(("HOLLOW_CRED_GITHUB_WHY", why.clone())),
        None => {}
    }
    match &creds.gcloud_mint {
        Some(Ok(argv)) => out.push((
            "HOLLOW_CRED_GCLOUD_MINT",
            serde_json::to_string(argv).unwrap_or_default(),
        )),
        Some(Err(why)) => out.push(("HOLLOW_CRED_GCLOUD_WHY", why.clone())),
        None => {}
    }
    // The access level comes back with each Google Cloud token, so the
    // runner's guard doesn't rest on a variable the agent could change.
    if let Some(level) = creds.gcloud_level {
        out.push(("HOLLOW_CRED_GCLOUD_LEVEL", level.as_str().to_string()));
    }
    out
}

/// The start line's words for what the turn can reach (no secret), or None
/// when it reaches nothing beyond its checkout.
pub(crate) fn notice(access: &TurnAccess) -> Option<serde_json::Value> {
    if !access.any() {
        return None;
    }
    let github = access.github.then(|| match github_source() {
        GithubSource::RunnerToken => {
            if matches!(crate::secret_store::present(GITHUB_TOKEN_KEY), Ok(true)) {
                "runner-token"
            } else {
                "no-token"
            }
        }
        GithubSource::GhLogin => {
            if gh_login_token().is_ok() {
                "gh-login"
            } else {
                "no-gh-login"
            }
        }
    });
    let cloud = (access.cloud != CloudAccess::Off).then(|| {
        if gcloud_program().is_none() {
            "no-gcloud".to_string()
        } else {
            access.cloud.as_str().to_string()
        }
    });
    Some(serde_json::json!({
        "github": github,
        "repo": access.gh_repo,
        "cloud": cloud,
        "serviceAccount": access.service_account,
    }))
}

/// What the agent is told about `access` (appended to the runner note).
pub(crate) fn agent_note(access: &TurnAccess) -> String {
    let mut out = String::new();
    if access.github {
        let repo = access
            .gh_repo
            .as_deref()
            .map(|r| format!(" `origin` there is https://github.com/{r}.git and `gh` targets {r}."))
            .unwrap_or_default();
        out.push_str(&format!(" GitHub is on for this turn: `git push` and `gh` (pull requests, merges) work there.{repo} Their credential comes from this Mac one call at a time; never print it, save it or set up another login there."));
    }
    match access.cloud {
        CloudAccess::Off => {}
        CloudAccess::Limited => out.push_str(" Google Cloud is on for this turn as the person's own account: use `gcloud` there. Production changes (Cloud Build approvals, Cloud Run deploys and traffic) are refused there; leave those to this Mac."),
        CloudAccess::Full => out.push_str(" Google Cloud is on for this turn as the person's own account, production changes included: use `gcloud` there, and confirm a production change with the person first."),
        CloudAccess::ServiceAccount => out.push_str(&format!(
            " Google Cloud is on for this turn as the service account {}: use `gcloud` there. Production changes (Cloud Build approvals, Cloud Run deploys and traffic) are refused there; leave those to this Mac.",
            access.service_account.as_deref().unwrap_or("(none named)")
        )),
    }
    out
}

#[cfg(test)]
#[path = "runner_access_tests.rs"]
pub(crate) mod tests;
