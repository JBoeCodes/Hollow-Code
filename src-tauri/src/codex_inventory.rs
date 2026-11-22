//! Codex's native MCP inventory (agent effectiveness F8, wave 3 · lane O3).
//!
//! Before Hollow starts Codex it asks the installed CLI which MCP servers
//! the user configured (`codex --disable apps --disable plugins --disable
//! remote_plugin mcp list --json`), so the run's command can switch every
//! one of them off (`agent::build_with_codex_servers`): Codex deep-merges
//! an empty `mcp_servers` override with its own config, so each configured
//! server has to be named. Only the server **names** come back from here;
//! the overrides, the preapprovals and everything else that depends on the
//! run's permission mode are built per run from the spec.
//!
//! That listing starts the CLI (a node wrapper and its native binary) on
//! every Codex command build. Since O3 an answer is reused while nothing it
//! depends on has changed ([`Cache`]): the key is
//! - the binary: its canonical path, size and modification time, and those
//!   of the `package.json` beside a node wrapper's folder (an npm upgrade
//!   rewrites it), so an upgrade lists again;
//! - `CODEX_HOME` (the app's, else `~/.codex`) and the working folder;
//! - the config revision: a SHA-256 over every file Codex reads MCP servers
//!   or their policy from: `config.toml`, `managed_config.toml` and
//!   `requirements.toml` in `CODEX_HOME` and in `/etc/codex`, the managed
//!   preferences (`com.openai.codex.plist`), and a project
//!   `.codex/config.toml` in the working folder or any folder above it.
//!   Each file counts by its contents (absent counts too; one over 1 MiB by
//!   its size and modification time).
//!
//! A changed key lists again. An entry also expires after [`TTL`], so an
//! input Codex reads that isn't hashed here can't keep a server switched
//! on for long, and at most [`MAX_ENTRIES`] are kept. A failed listing is
//! never kept (the run fails closed, and the next one asks again).
//! **Permissions are never cached**: the entry is the names alone, and each
//! run builds its own overrides and approvals for its own mode. The cache
//! lives in process memory: a restart, or the `hollow team-mcp` process
//! (its own process), starts empty.
use crate::agent::{ProcessScope, RunSpec};
use crate::process_group;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime};

/// How long a listing is reused at most, even when nothing hashed changed.
pub(crate) const TTL: Duration = Duration::from_secs(10 * 60);
/// Listings kept at most (the oldest goes first).
pub(crate) const MAX_ENTRIES: usize = 16;
/// A config file larger than this counts by its size and modification
/// time instead of its contents.
const MAX_HASHED_BYTES: u64 = 1024 * 1024;

/// Whether a listing ran `codex mcp list` or reused an earlier one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Served {
    /// The CLI was asked (a first listing, a changed binary or config, an
    /// expired entry, or a failure, which is never kept).
    Listed,
    /// An identical earlier listing was reused: nothing was started.
    Cached,
}

impl Served {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Served::Listed => "listed",
            Served::Cached => "cached",
        }
    }
}

/// The configured server names (or why they couldn't be read), and how
/// they were served.
#[derive(Debug, Clone)]
pub(crate) struct Listing {
    pub servers: Result<Vec<String>, String>,
    pub served: Served,
}

/// The inventory step of one command build, for the run manifest: its
/// mark on the preparation timeline and how it was served.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Inventory {
    pub mark: crate::prep_timing::Mark,
    pub served: Served,
}

/// Where a listing comes from: the CLI at `bin`, with `CODEX_HOME` set to
/// `home` when given (otherwise the CLI's own default, as the agent run
/// gets it).
#[derive(Debug, Clone)]
pub(crate) struct Cli {
    pub bin: PathBuf,
    pub home: Option<PathBuf>,
}

impl Cli {
    /// The installed CLI, as `agent::find_binary` resolves it, with the
    /// app's own `CODEX_HOME`, if it has one.
    pub(crate) fn installed() -> Self {
        Cli {
            bin: crate::agent::find_binary(crate::agent::AgentKind::Codex)
                .unwrap_or_else(|| PathBuf::from("codex")),
            home: std::env::var_os("CODEX_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        }
    }
}

/// One file's identity: canonical path, size and modification time.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FileId {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}

impl FileId {
    fn of(path: &Path) -> Option<Self> {
        let path = std::fs::canonicalize(path).ok()?;
        let meta = std::fs::metadata(&path).ok()?;
        Some(FileId {
            size: meta.len(),
            modified: meta.modified().ok(),
            path,
        })
    }
}

/// What a listing depends on (see the module doc).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    bin: Option<FileId>,
    /// The binary as given, when it doesn't resolve (a bare `codex`).
    bin_name: PathBuf,
    package: Option<FileId>,
    home: PathBuf,
    cwd: PathBuf,
    config: [u8; 32],
}

/// The files Codex reads its MCP servers (or their policy) from, for a run
/// in `cwd` with `home` as `CODEX_HOME`.
fn config_files(home: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in [home, Path::new("/etc/codex")] {
        for name in ["config.toml", "managed_config.toml", "requirements.toml"] {
            files.push(dir.join(name));
        }
    }
    let managed = Path::new("/Library/Managed Preferences");
    files.push(managed.join("com.openai.codex.plist"));
    if let Some(user) = std::env::var_os("USER").filter(|u| !u.is_empty()) {
        files.push(managed.join(user).join("com.openai.codex.plist"));
    }
    for dir in cwd.ancestors() {
        files.push(dir.join(".codex").join("config.toml"));
    }
    files
}

/// The config revision: every file in [`config_files`], by contents.
fn config_revision(home: &Path, cwd: &Path) -> [u8; 32] {
    let mut hash = Sha256::new();
    for file in config_files(home, cwd) {
        hash.update(file.as_os_str().as_encoded_bytes());
        hash.update([0]);
        match std::fs::metadata(&file) {
            Err(_) => hash.update(b"absent"),
            Ok(meta) if meta.len() > MAX_HASHED_BYTES => {
                hash.update(b"large");
                hash.update(meta.len().to_le_bytes());
                hash.update(format!("{:?}", meta.modified().ok()).as_bytes());
            }
            Ok(_) => match std::fs::read(&file) {
                Ok(bytes) => {
                    hash.update(b"file");
                    hash.update(Sha256::digest(&bytes));
                }
                Err(e) => hash.update(format!("unreadable {:?}", e.kind()).as_bytes()),
            },
        }
        hash.update([0]);
    }
    hash.finalize().into()
}

/// The `CODEX_HOME` a listing reads: the given one, else `~/.codex`.
fn home_of(cli: &Cli) -> PathBuf {
    cli.home
        .clone()
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
        .unwrap_or_default()
}

fn key(cli: &Cli, cwd: &Path) -> Key {
    let bin = FileId::of(&cli.bin);
    // A node wrapper (`…/@openai/codex/bin/codex.js`): its package's
    // manifest, which an npm upgrade rewrites even when the wrapper is the
    // same file.
    let package = bin
        .as_ref()
        .and_then(|b| b.path.parent()?.parent().map(|p| p.join("package.json")))
        .and_then(|p| FileId::of(&p));
    let home = home_of(cli);
    Key {
        bin,
        bin_name: cli.bin.clone(),
        package,
        config: config_revision(&home, cwd),
        home,
        cwd: cwd.to_path_buf(),
    }
}

struct Entry {
    names: Vec<String>,
    at: Instant,
}

/// Listings, keyed by what they depend on (see the module doc).
#[derive(Default)]
pub(crate) struct Cache {
    entries: std::sync::Mutex<HashMap<Key, Entry>>,
}

impl Cache {
    pub(crate) fn new() -> Self {
        Cache::default()
    }

    /// The configured servers for a run of `spec` through `cli`: a kept
    /// listing for the same binary, config and folder when there is one
    /// younger than [`TTL`], else a new listing (kept only when it
    /// succeeded). The lock is never held while the CLI runs.
    pub(crate) fn servers(&self, cli: &Cli, spec: &RunSpec) -> Listing {
        let key = key(cli, &spec.cwd);
        if let Ok(entries) = self.entries.lock() {
            if let Some(entry) = entries.get(&key).filter(|e| e.at.elapsed() < TTL) {
                return Listing {
                    servers: Ok(entry.names.clone()),
                    served: Served::Cached,
                };
            }
        }
        let servers = list(cli, spec);
        if let (Ok(names), Ok(mut entries)) = (&servers, self.entries.lock()) {
            entries.retain(|_, e| e.at.elapsed() < TTL);
            if entries.len() >= MAX_ENTRIES && !entries.contains_key(&key) {
                if let Some(oldest) = entries
                    .iter()
                    .min_by_key(|(_, e)| e.at)
                    .map(|(k, _)| k.clone())
                {
                    entries.remove(&oldest);
                }
            }
            entries.insert(
                key,
                Entry {
                    names: names.clone(),
                    at: Instant::now(),
                },
            );
        }
        Listing {
            servers,
            served: Served::Listed,
        }
    }

    /// Age every entry by `by` (tests: the TTL without waiting).
    #[cfg(test)]
    fn age(&self, by: Duration) {
        for entry in self.entries.lock().unwrap().values_mut() {
            entry.at = entry.at.checked_sub(by).unwrap_or(entry.at);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
}

/// The app's cache: one per process.
static APP: once_cell::sync::Lazy<Cache> = once_cell::sync::Lazy::new(Cache::new);

/// The installed CLI's configured servers for `spec`, through the app's
/// cache (what `agent::build_command` uses). Self-hosted models Q9: a
/// self-hosted Codex run lists against the entry's own `CODEX_HOME`, the
/// one its run gets (made first; a home that can't be used fails the
/// listing, so the run fails closed).
pub(crate) fn installed(spec: &RunSpec) -> Listing {
    let mut cli = Cli::installed();
    if let Some(target) = spec.extras.self_hosted.as_ref() {
        // Q9 review: a target Codex can't run (an `anthropic` entry, or no
        // home) is refused by the builder; nothing is listed for it, so
        // `codex mcp list` never reads the person's own `~/.codex` first.
        let home = match (target.agent(), target.codex_home.clone()) {
            (Some(crate::agent::AgentKind::Codex), Some(home)) => home,
            _ => {
                return Listing {
                    servers: Err(crate::self_hosted::wrong_harness(
                        target,
                        crate::agent::AgentKind::Codex,
                    )
                    .unwrap_or_else(|| {
                        format!(
                            "The self-hosted model {} has no Codex home in Hollow's app data, so the run didn't start.",
                            target.name
                        )
                    })),
                    served: Served::Listed,
                }
            }
        };
        if let Err(why) = crate::self_hosted::ensure_codex_home(&home) {
            return Listing {
                servers: Err(why),
                served: Served::Listed,
            };
        }
        cli.home = Some(home);
    }
    APP.servers(&cli, spec)
}

/// Ask the CLI. Bounded: 5 s for the listing, 2 MiB of output, and the
/// whole process group ended for a GUI-owned run.
pub(crate) fn list(cli: &Cli, spec: &RunSpec) -> Result<Vec<String>, String> {
    use std::io::Read;
    let mut probe = std::process::Command::new(&cli.bin);
    probe
        .args([
            "--disable",
            "apps",
            "--disable",
            "plugins",
            "--disable",
            "remote_plugin",
            "mcp",
            "list",
            "--json",
        ])
        .current_dir(&spec.cwd)
        .env("PATH", crate::agent::cli_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(home) = &cli.home {
        probe.env("CODEX_HOME", home);
    }
    // `codex` is a node wrapper whose native binary shares the wrapper's
    // group, so killing only the wrapper on timeout would orphan the native
    // listing and leave it holding the pipe. A GUI-owned run therefore gives
    // the probe its own group and ends that whole group. Inside `hollow
    // team-mcp` (an inherited scope) no group may be created, so there only
    // the bounded read below keeps a leftover from wedging the caller.
    let isolated = spec.process_scope == ProcessScope::GuiOwned;
    if isolated {
        process_group::isolate_probe(&mut probe);
    }
    let mut child = probe
        .spawn()
        .map_err(|_| "Could not isolate Codex connectors: local CLI unavailable.".to_string())?;
    let stdout = child.stdout.take().expect("piped configuration listing");
    let (listing_tx, listing) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut data = Vec::new();
        let result = stdout.take(2 * 1024 * 1024).read_to_end(&mut data);
        let _ = listing_tx.send(result.map(|_| data));
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => {
                if isolated {
                    process_group::end_probe_group(child.id());
                }
                let _ = child.kill();
                let _ = child.wait();
                break Err(
                    "Could not isolate Codex connectors: local configuration listing timed out."
                        .to_string(),
                );
            }
        }
    };
    if isolated {
        // Nothing the listing started may outlive it or hold its pipe open.
        process_group::end_probe_group(child.id());
    }
    let raw = listing
        .recv_timeout(std::time::Duration::from_secs(2))
        .ok()
        .and_then(Result::ok);
    let status = status?;
    let raw = raw.ok_or_else(|| {
        "Could not isolate Codex connectors: local configuration listing unreadable.".to_string()
    })?;
    if !status.success() {
        return Err(
            "Could not isolate Codex connectors: local configuration listing failed.".into(),
        );
    }
    crate::agent::codex_server_names(&raw)
}

#[cfg(test)]
pub(crate) mod fake {
    //! A stand-in `codex` for tests and the preparation bench: it answers
    //! `mcp list --json` with `$CODEX_HOME/fake-servers.json` and appends a
    //! line to `$CODEX_HOME/fake-listings.log` each time it is started.
    use std::path::{Path, PathBuf};

    pub(crate) const SERVERS: &str = "fake-servers.json";
    pub(crate) const LOG: &str = "fake-listings.log";

    /// Write the fake CLI into `dir` (a `/bin/sh` script) and return it.
    pub(crate) fn shell(dir: &Path) -> PathBuf {
        script(
            dir,
            "codex",
            "#!/bin/sh\necho listed >> \"$CODEX_HOME/fake-listings.log\"\ncat \"$CODEX_HOME/fake-servers.json\"\n",
        )
    }

    /// The same, shaped like the real CLI: a `node` wrapper that starts a
    /// separate native child, which answers. `None` when `node` isn't on
    /// the app's PATH.
    pub(crate) fn node_wrapper(dir: &Path) -> Option<PathBuf> {
        let native = shell(dir);
        let path = crate::agent::cli_path();
        let node = path
            .split(':')
            .map(|d| Path::new(d).join("node"))
            .find(|p| p.is_file())?;
        Some(script(
            dir,
            "codex.js",
            &format!(
                "#!{}\nconst r = require('child_process').spawnSync({:?}, process.argv.slice(2), {{ stdio: 'inherit' }});\nprocess.exit(r.status ?? 1);\n",
                node.display(),
                native.display().to_string()
            ),
        ))
    }

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Set the servers the fake reports (as `codex mcp list --json` does).
    pub(crate) fn set_servers(home: &Path, names: &[&str]) {
        let entries: Vec<serde_json::Value> = names
            .iter()
            .map(|n| serde_json::json!({"name": n, "enabled": true}))
            .collect();
        std::fs::write(home.join(SERVERS), serde_json::to_vec(&entries).unwrap()).unwrap();
    }

    /// How many times the fake was started.
    pub(crate) fn listings(home: &Path) -> usize {
        std::fs::read_to_string(home.join(LOG))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentKind, RunExtras};

    fn spec(cwd: &Path, perm: &str) -> RunSpec {
        RunSpec {
            prompt: "hello".into(),
            resume: None,
            model: None,
            perm: perm.into(),
            cwd: cwd.to_path_buf(),
            allowed_tools: vec![],
            disallowed_tools: vec![],
            interactive: true,
            mcp_config: None,
            append_system_prompt: None,
            process_scope: ProcessScope::GuiOwned,
            usage_stream: false,
            max_budget_usd: None,
            extras: RunExtras::default(),
        }
    }

    /// A fake CLI with its own `CODEX_HOME` and a working folder.
    struct Setup {
        _dir: tempfile::TempDir,
        cli: Cli,
        home: PathBuf,
        cwd: PathBuf,
    }

    fn setup(names: &[&str]) -> Setup {
        let dir = tempfile::Builder::new()
            .prefix("hollow-codex-inventory-")
            .tempdir()
            .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let home = root.join("codex-home");
        let cwd = root.join("work/project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(
            home.join("config.toml"),
            "[mcp_servers.mail]\ncommand = \"mail\"\n",
        )
        .unwrap();
        fake::set_servers(&home, names);
        let bin = fake::shell(&root.join("work"));
        Setup {
            _dir: dir,
            cli: Cli {
                bin,
                home: Some(home.clone()),
            },
            home,
            cwd,
        }
    }

    fn names(listing: &Listing) -> Vec<String> {
        listing.servers.clone().expect("listed")
    }

    /// Agent effectiveness F8 (O3, red first): an identical run reuses the
    /// listing (nothing is started), and a change to Codex's config file
    /// invalidates it: the next run lists again and sees the new server.
    #[test]
    fn identical_runs_reuse_the_listing_and_a_config_change_invalidates_it() {
        let s = setup(&["mail"]);
        let cache = Cache::new();
        let run = spec(&s.cwd, "acceptEdits");
        let first = cache.servers(&s.cli, &run);
        assert_eq!(first.served, Served::Listed);
        assert_eq!(names(&first), ["mail"]);
        let second = cache.servers(&s.cli, &run);
        assert_eq!(second.served, Served::Cached, "an identical run reuses it");
        assert_eq!(names(&second), ["mail"]);
        assert_eq!(fake::listings(&s.home), 1, "the CLI was started once");

        // The user adds a server: the config file changes.
        std::fs::write(
            s.home.join("config.toml"),
            "[mcp_servers.mail]\ncommand = \"mail\"\n[mcp_servers.slack]\ncommand = \"slack\"\n",
        )
        .unwrap();
        fake::set_servers(&s.home, &["mail", "slack"]);
        let third = cache.servers(&s.cli, &run);
        assert_eq!(third.served, Served::Listed, "the config changed");
        assert_eq!(names(&third), ["mail", "slack"]);
        assert_eq!(fake::listings(&s.home), 2);
        assert_eq!(cache.servers(&s.cli, &run).served, Served::Cached);
    }

    /// Every other input invalidates it too: a project `.codex/config.toml`
    /// in the working folder or above it, Codex's managed and requirements
    /// files beside its config, a different `CODEX_HOME`, a different
    /// working folder, and a changed binary (size or modification time).
    #[test]
    fn project_config_home_folder_and_binary_changes_invalidate_it() {
        let s = setup(&["mail"]);
        let cache = Cache::new();
        let run = spec(&s.cwd, "acceptEdits");
        let listed = |cache: &Cache, cli: &Cli, run: &RunSpec| cache.servers(cli, run).served;
        assert_eq!(listed(&cache, &s.cli, &run), Served::Listed);
        assert_eq!(listed(&cache, &s.cli, &run), Served::Cached);

        // A project config above the working folder.
        let project = s.cwd.parent().unwrap().join(".codex");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("config.toml"),
            "[mcp_servers.repo]\ncommand = \"r\"\n",
        )
        .unwrap();
        assert_eq!(
            listed(&cache, &s.cli, &run),
            Served::Listed,
            "project config"
        );
        assert_eq!(listed(&cache, &s.cli, &run), Served::Cached);

        // Codex's managed config beside its own.
        std::fs::write(s.home.join("managed_config.toml"), "model = \"x\"\n").unwrap();
        assert_eq!(
            listed(&cache, &s.cli, &run),
            Served::Listed,
            "managed config"
        );
        std::fs::write(s.home.join("requirements.toml"), "x = 1\n").unwrap();
        assert_eq!(listed(&cache, &s.cli, &run), Served::Listed, "requirements");
        assert_eq!(listed(&cache, &s.cli, &run), Served::Cached);

        // Another working folder.
        let other = s.cwd.parent().unwrap().join("other");
        std::fs::create_dir_all(&other).unwrap();
        assert_eq!(
            listed(&cache, &s.cli, &spec(&other, "acceptEdits")),
            Served::Listed,
            "another folder"
        );

        // Another CODEX_HOME (a copy of the same files).
        let home2 = s.home.parent().unwrap().join("codex-home-2");
        std::fs::create_dir_all(&home2).unwrap();
        for f in ["config.toml", fake::SERVERS] {
            std::fs::copy(s.home.join(f), home2.join(f)).unwrap();
        }
        let cli2 = Cli {
            bin: s.cli.bin.clone(),
            home: Some(home2),
        };
        assert_eq!(listed(&cache, &cli2, &run), Served::Listed, "another home");

        // The binary is replaced (an upgrade): new size and mtime.
        assert_eq!(listed(&cache, &s.cli, &run), Served::Cached);
        let body = std::fs::read_to_string(&s.cli.bin).unwrap();
        std::fs::write(&s.cli.bin, format!("{body}# 0.154.0\n")).unwrap();
        assert_eq!(listed(&cache, &s.cli, &run), Served::Listed, "a new binary");
        assert_eq!(listed(&cache, &s.cli, &run), Served::Cached);
        // Same size, newer modification time.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&s.cli.bin)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(
            listed(&cache, &s.cli, &run),
            Served::Listed,
            "a newer binary"
        );
    }

    /// The manifest says how each Codex run's inventory was served and
    /// times it on the timeline (`codexInventory`); another agent's run
    /// records neither.
    #[test]
    fn the_manifest_records_how_the_inventory_was_served() {
        use crate::prep_timing::Step;
        let s = setup(&["mail"]);
        let cache = Cache::new();
        let run = spec(&s.cwd, "acceptEdits");
        let mut served = Vec::new();
        for _ in 0..2 {
            let (_cmd, inventory) =
                crate::agent::build_command_via(AgentKind::Codex, &run, |spec| {
                    cache.servers(&s.cli, spec)
                });
            let mut manifest = crate::run_manifest::RunManifest::new();
            manifest.set_codex_inventory(inventory);
            assert!(manifest.timer.0.get(Step::CodexInventory).is_some());
            let json = serde_json::to_value(&manifest).unwrap();
            served.push(json["preparation"]["codexInventory"].clone());
        }
        assert_eq!(
            served,
            [serde_json::json!("listed"), serde_json::json!("cached")]
        );
        let (_cmd, none) = crate::agent::build_command_via(AgentKind::Claude, &run, |_| {
            unreachable!("only Codex lists")
        });
        let mut manifest = crate::run_manifest::RunManifest::new();
        manifest.set_codex_inventory(none);
        assert!(manifest.timer.0.get(Step::CodexInventory).is_none());
        let json = serde_json::to_value(&manifest).unwrap();
        assert!(json["preparation"].get("codexInventory").is_none());
    }

    /// An entry expires after the TTL even when nothing hashed changed,
    /// and at most [`MAX_ENTRIES`] folders are kept.
    #[test]
    fn entries_expire_and_are_bounded() {
        let s = setup(&["mail"]);
        let cache = Cache::new();
        let run = spec(&s.cwd, "acceptEdits");
        assert_eq!(cache.servers(&s.cli, &run).served, Served::Listed);
        cache.age(TTL - Duration::from_secs(1));
        assert_eq!(cache.servers(&s.cli, &run).served, Served::Cached);
        cache.age(Duration::from_secs(2));
        assert_eq!(
            cache.servers(&s.cli, &run).served,
            Served::Listed,
            "expired"
        );
        for i in 0..MAX_ENTRIES + 4 {
            let cwd = s.cwd.join(format!("f{i}"));
            std::fs::create_dir_all(&cwd).unwrap();
            cache.servers(&s.cli, &spec(&cwd, "acceptEdits"));
        }
        assert_eq!(cache.len(), MAX_ENTRIES);
    }

    /// A failed listing is never kept: the next run asks again, and once
    /// the CLI answers, that answer is what is reused.
    #[test]
    fn failures_are_never_cached() {
        let s = setup(&["mail"]);
        let cache = Cache::new();
        let run = spec(&s.cwd, "acceptEdits");
        std::fs::write(s.home.join(fake::SERVERS), "not json").unwrap();
        let failed = cache.servers(&s.cli, &run);
        assert!(failed.servers.is_err());
        let again = cache.servers(&s.cli, &run);
        assert_eq!(again.served, Served::Listed, "a failure is not reused");
        assert!(again.servers.is_err());
        fake::set_servers(&s.home, &["mail"]);
        assert_eq!(cache.servers(&s.cli, &run).served, Served::Listed);
        assert_eq!(cache.servers(&s.cli, &run).served, Served::Cached);
        assert_eq!(fake::listings(&s.home), 3);
    }

    /// Permissions are never cached: only names are kept, so two runs in
    /// different modes that share a listing still get their own approvals,
    /// and both switch every configured server off.
    #[test]
    fn a_shared_listing_never_carries_a_runs_permissions() {
        let s = setup(&["mail", "custom.slack"]);
        let cache = Cache::new();
        let mut plan = spec(&s.cwd, "plan");
        plan.process_scope = ProcessScope::Inherited;
        let mut bypass = plan.clone();
        bypass.perm = "bypassPermissions".into();
        let render = |run: &RunSpec| {
            let (cmd, inventory) = crate::agent::build_command_via(AgentKind::Codex, run, |spec| {
                cache.servers(&s.cli, spec)
            });
            let args: Vec<String> = cmd
                .as_std()
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            (args.join(" "), inventory.expect("codex lists").served)
        };
        let (plan_args, first) = render(&plan);
        let (bypass_args, second) = render(&bypass);
        assert_eq!((first, second), (Served::Listed, Served::Cached));
        for args in [&plan_args, &bypass_args] {
            assert!(args.contains("\"mail\"={enabled=false}"), "{args}");
            assert!(args.contains("\"custom.slack\"={enabled=false}"), "{args}");
        }
        assert!(plan_args.contains("-s read-only"), "{plan_args}");
        assert!(!plan_args.contains("--dangerously-bypass-approvals-and-sandbox"));
        assert!(bypass_args.contains("--dangerously-bypass-approvals-and-sandbox"));
        assert!(!bypass_args.contains("-s read-only"));
    }
}
