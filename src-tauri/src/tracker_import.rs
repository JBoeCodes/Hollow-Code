//! `hollow tracker-import`: the one-time Notion-to-vault importer and the
//! tracker rekey (design `linear-source-of-truth-2026-11-07` sections 5 and
//! 8; board rows M1 and M2). Contract: `docs/TRACKER_SYNC.md`, "The
//! importer".
//!
//! ```text
//! hollow tracker-import --config <config.json> --vault <vault>
//!     (--preflight --staging <dir>
//!     | --dry-run --staging <dir> [--mapping <file.md>]...
//!     | --apply --backup-dir <dir>
//!     | --rekey --backup-dir <dir> [--mapping <file.md>]...
//!     | --rollback <manifest.json>)
//!     [--member <name>] [--bind-project <project key or id>=<workspace dir>]...
//! ```
//!
//! - **Read only outside the vault.** Notion goes through the importer's own
//!   client ([`notion`]), which can only GET and run the data-source query.
//!   The tracker goes through the existing adapter (`tracker::adapter`, its
//!   team scope guard included) and only its read methods are called. The
//!   importer never creates, edits or archives anything in either service.
//! - **Every vault write is journaled.** `--apply` and `--rekey` save each
//!   changed file's bytes under `--backup-dir/<stamp>/` before writing and
//!   record every path created, changed or moved with its SHA-256 values in
//!   that folder's `manifest.json`; `--rollback` puts every byte back and
//!   checks every hash ([`files`]). A second run changes nothing.
//! - **Secrets stay out.** The Notion token (`notion.token`, or the legacy
//!   `notionToken`) and the tracker key are never printed, logged, passed in
//!   argv or kept in an error: every message goes through [`scrub`].

pub(crate) mod files;
mod notion;
mod records;
mod rekey;
#[cfg(test)]
mod tests;

use crate::tracker::{self, TrackerAdapter};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: hollow tracker-import --config <config.json> --vault <vault> (--preflight --staging <dir> | --dry-run --staging <dir> [--mapping <file.md>]... | --apply --backup-dir <dir> | --rekey --backup-dir <dir> [--mapping <file.md>]... | --rollback <manifest.json>) [--member <name>] [--bind-project <project key or id>=<workspace dir>]...";

/// The team the tracker block names when the config has none yet (design
/// section 4: team Engineering).
pub(crate) const DEFAULT_TEAM_ID: &str = "11111111-1111-4111-8111-111111111111";
pub(crate) const DEFAULT_TEAM_KEY: &str = "ENG";
/// A Working File over this is skipped and reported.
pub(crate) const FILE_CAP: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    Preflight,
    DryRun,
    Apply,
    Rekey,
    Rollback(PathBuf),
}

impl Mode {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Mode::Preflight => "preflight",
            Mode::DryRun => "dry-run",
            Mode::Apply => "apply",
            Mode::Rekey => "rekey",
            Mode::Rollback(_) => "rollback",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Args {
    pub config: PathBuf,
    pub vault: PathBuf,
    pub staging: Option<PathBuf>,
    pub mapping: Vec<PathBuf>,
    pub backup_dir: Option<PathBuf>,
    pub member: String,
    /// `--bind-project KEY=DIR`, repeatable: the tracker project (key like
    /// `P-ENG-22`, or id) goes to `workspaces/DIR`, over every other rule.
    pub bind_projects: Vec<(String, String)>,
    pub mode: Mode,
}

pub(crate) fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut config = None;
    let mut vault = None;
    let mut staging = None;
    let mut mapping = Vec::new();
    let mut backup_dir = None;
    let mut member = None;
    let mut bind_projects = Vec::new();
    let mut modes = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .filter(|v| !v.starts_with("--"))
                .map(PathBuf::from)
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--config" => config = Some(value("--config")?),
            "--vault" => vault = Some(value("--vault")?),
            "--staging" => staging = Some(value("--staging")?),
            "--mapping" => mapping.push(value("--mapping")?),
            "--backup-dir" => backup_dir = Some(value("--backup-dir")?),
            "--member" => member = Some(value("--member")?.to_string_lossy().to_string()),
            "--bind-project" => {
                let pair = value("--bind-project")?.to_string_lossy().to_string();
                let (project, dir) = pair
                    .split_once('=')
                    .map(|(p, d)| {
                        (
                            p.trim(),
                            d.trim()
                                .trim_start_matches("workspaces/")
                                .trim_end_matches('/'),
                        )
                    })
                    .filter(|(p, d)| !p.is_empty() && !d.is_empty())
                    .ok_or("--bind-project needs <project key or id>=<workspace dir>")?;
                bind_projects.push((project.to_string(), dir.to_string()));
            }
            "--preflight" => modes.push(Mode::Preflight),
            "--dry-run" => modes.push(Mode::DryRun),
            "--apply" => modes.push(Mode::Apply),
            "--rekey" => modes.push(Mode::Rekey),
            "--rollback" => modes.push(Mode::Rollback(value("--rollback")?)),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if modes.len() != 1 {
        return Err(
            "choose exactly one of --preflight, --dry-run, --apply, --rekey, --rollback".into(),
        );
    }
    let mode = modes.remove(0);
    let config = config.ok_or("--config is required")?;
    let vault = vault.ok_or("--vault is required")?;
    match mode {
        Mode::Preflight | Mode::DryRun if staging.is_none() => {
            return Err(format!("--{} needs --staging", mode.name()))
        }
        Mode::Apply | Mode::Rekey if backup_dir.is_none() => {
            return Err(format!("--{} needs --backup-dir", mode.name()))
        }
        _ => {}
    }
    Ok(Args {
        config,
        vault,
        staging,
        mapping,
        backup_dir,
        member: member.unwrap_or_else(|| "Sam Taylor".into()),
        bind_projects,
        mode,
    })
}

/// What the importer reads from Hollow's config file. `Debug` is written
/// by hand so the token never shows.
#[derive(Clone)]
pub(crate) struct Settings {
    pub notion_token: Option<String>,
    pub notion_version: String,
    pub two_way_sync: bool,
    pub sources: BTreeMap<records::Kind, String>,
    pub tracker: tracker::settings::TrackerConfig,
    /// The tracker block was absent and the design's team was assumed.
    pub tracker_assumed: bool,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field(
                "notion_token",
                &self.notion_token.as_ref().map(|_| "[redacted]"),
            )
            .field("notion_version", &self.notion_version)
            .field("two_way_sync", &self.two_way_sync)
            .field("sources", &self.sources)
            .field("tracker", &self.tracker)
            .field("tracker_assumed", &self.tracker_assumed)
            .finish()
    }
}

fn text_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |v, key| v.get(*key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

impl Settings {
    pub(crate) fn from_value(config: &Value) -> Settings {
        let notion_token = text_at(config, &["notion", "token"])
            .or_else(|| text_at(config, &["notionToken"]))
            .map(str::to_string);
        let mut sources = BTreeMap::new();
        for kind in records::Kind::ALL {
            let configured = text_at(
                config,
                &["trackerImport", "notionSources", kind.config_key()],
            )
            .or_else(|| match kind {
                records::Kind::Projects => text_at(config, &["notion", "projectDataSourceId"]),
                records::Kind::Tasks => text_at(config, &["notion", "taskDataSourceId"]),
                _ => None,
            })
            .map(str::to_string)
            .or_else(|| kind.default_source().map(str::to_string));
            if let Some(id) = configured {
                sources.insert(kind, id);
            }
        }
        let mut tracker = tracker::settings::TrackerConfig::from_value(
            config.get("tracker").unwrap_or(&Value::Null),
        );
        let tracker_assumed = tracker.provider.is_none();
        if tracker_assumed {
            tracker.provider = Some(tracker::linear::PROVIDER.into());
            tracker.team_id = Some(DEFAULT_TEAM_ID.into());
            tracker.team_key = Some(DEFAULT_TEAM_KEY.into());
        }
        Settings {
            notion_token,
            notion_version: text_at(config, &["notion", "apiVersion"])
                .unwrap_or("2026-03-11")
                .to_string(),
            two_way_sync: config
                .pointer("/notion/twoWaySync")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            sources,
            tracker,
            tracker_assumed,
        }
    }

    pub(crate) fn load(path: &Path) -> Result<Settings, String> {
        let raw =
            std::fs::read(path).map_err(|_| "The config file couldn't be read.".to_string())?;
        let value: Value =
            serde_json::from_slice(&raw).map_err(|_| "The config file isn't JSON.".to_string())?;
        Ok(Settings::from_value(&value))
    }

    pub(crate) fn provider(&self) -> &str {
        self.tracker
            .provider
            .as_deref()
            .unwrap_or(tracker::linear::PROVIDER)
    }

    pub(crate) fn scope(&self) -> &str {
        self.tracker.team_id.as_deref().unwrap_or(DEFAULT_TEAM_ID)
    }

    /// The importer works for team ENG only: the config's team must be its
    /// id, and its key (when set) `ENG`.
    pub(crate) fn check_scope(&self) -> Result<(), String> {
        if self.scope() != DEFAULT_TEAM_ID
            || self
                .tracker
                .team_key
                .as_deref()
                .is_some_and(|key| key != DEFAULT_TEAM_KEY)
        {
            return Err(format!(
                "The importer works only for team {DEFAULT_TEAM_KEY} ({DEFAULT_TEAM_ID}); the config names another team."
            ));
        }
        Ok(())
    }
}

/// The tracker key: Hollow's own bundle first, then the provider's own
/// Keychain item (read, never copied). Never printed.
fn tracker_key(provider: &str) -> Option<String> {
    if let Ok(Some(key)) = tracker::settings::saved_key(provider) {
        return Some(key);
    }
    let (service, account) = tracker::settings::provider(provider)?.keychain_item?;
    crate::secret_store::read_item(service, account)
        .ok()
        .flatten()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

/// `text` with every secret, and anything shaped like a Notion or tracker
/// token, replaced by `[redacted]`.
pub(crate) fn scrub(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        let secret = secret.trim();
        if secret.len() >= 6 {
            out = out.replace(secret, "[redacted]");
        }
    }
    for prefix in ["secret_", "ntn_", "lin_api_", "lin_oauth_"] {
        let mut from = 0;
        while let Some(at) = out[from..].find(prefix).map(|i| i + from) {
            let end = out[at + prefix.len()..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .map_or(out.len(), |i| i + at + prefix.len());
            if end - at - prefix.len() >= 8 {
                out.replace_range(at..end, "[redacted]");
                from = at + "[redacted]".len();
            } else {
                from = end;
            }
        }
    }
    out
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// One row of a report list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Item {
    pub path: String,
    pub title: String,
    pub detail: String,
}

impl Item {
    pub(crate) fn new(path: &str, title: &str, detail: impl Into<String>) -> Item {
        Item {
            path: path.into(),
            title: title.into(),
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceInfo {
    pub id: String,
    pub title: String,
    pub rows: usize,
}

/// What a run found and planned: `report.json` and `report.md`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Report {
    pub mode: String,
    pub sources: BTreeMap<String, SourceInfo>,
    pub projects_notion: usize,
    pub projects_tracked: usize,
    pub projects_vault_only: usize,
    /// Each tracked project's workspace and why it was chosen.
    pub projects_bound: Vec<Item>,
    pub tracker_projects_without_workspace: Vec<Item>,
    pub open_tasks: usize,
    pub open_mapped: usize,
    pub open_matched: Vec<Item>,
    pub open_need_issue: Vec<Item>,
    pub mapping_rows: usize,
    pub mapping_not_applied: Vec<Item>,
    pub split_files: Vec<Item>,
    pub done_tasks: usize,
    pub task_files_created: usize,
    pub tracker_issues: usize,
    pub tracker_issue_files_new: usize,
    pub dropped_out_of_scope: usize,
    pub meetings: usize,
    pub people: usize,
    pub people_in_both: usize,
    pub domains: usize,
    pub war_room: usize,
    pub captures: usize,
    pub docs: usize,
    pub ai_subscription_rows: usize,
    pub working_files: usize,
    pub working_files_bytes: u64,
    pub working_files_skipped: Vec<Item>,
    pub archive_moves: Vec<String>,
    pub files_created: usize,
    pub files_changed: usize,
    pub errors: Vec<String>,
}

impl Report {
    fn markdown(&self) -> String {
        let mut out = format!("# Tracker import, {}\n\n", self.mode);
        let line = |out: &mut String, label: &str, value: String| {
            out.push_str(&format!("- {label}: {value}\n"));
        };
        out.push_str("## Counts\n\n");
        for (kind, info) in &self.sources {
            line(
                &mut out,
                &format!("Notion source {kind}"),
                format!("\"{}\" ({}), {} rows", info.title, info.id, info.rows),
            );
        }
        line(
            &mut out,
            "Projects in Notion",
            self.projects_notion.to_string(),
        );
        line(
            &mut out,
            "Projects tracked",
            self.projects_tracked.to_string(),
        );
        line(
            &mut out,
            "Projects vault-only",
            self.projects_vault_only.to_string(),
        );
        line(&mut out, "Open tasks", self.open_tasks.to_string());
        line(
            &mut out,
            "Open tasks mapped (mapping rows)",
            self.open_mapped.to_string(),
        );
        line(
            &mut out,
            "Open tasks matched (description or title)",
            self.open_matched.len().to_string(),
        );
        line(
            &mut out,
            "Open tasks that need an issue",
            self.open_need_issue.len().to_string(),
        );
        line(&mut out, "Mapping rows read", self.mapping_rows.to_string());
        line(
            &mut out,
            "Mapping rows not applied",
            self.mapping_not_applied.len().to_string(),
        );
        line(&mut out, "Done tasks frozen", self.done_tasks.to_string());
        line(
            &mut out,
            "Task files created from Notion",
            self.task_files_created.to_string(),
        );
        line(
            &mut out,
            "Tracker issues in the team",
            self.tracker_issues.to_string(),
        );
        line(
            &mut out,
            "New issue files",
            self.tracker_issue_files_new.to_string(),
        );
        line(
            &mut out,
            "Out-of-scope nodes dropped",
            self.dropped_out_of_scope.to_string(),
        );
        line(&mut out, "Meetings", self.meetings.to_string());
        line(
            &mut out,
            "People",
            format!("{} ({} in both sources)", self.people, self.people_in_both),
        );
        line(&mut out, "Domains", self.domains.to_string());
        line(&mut out, "War Room rows", self.war_room.to_string());
        line(&mut out, "Captures", self.captures.to_string());
        line(&mut out, "Docs", self.docs.to_string());
        line(
            &mut out,
            "AI Subscription rows",
            self.ai_subscription_rows.to_string(),
        );
        line(
            &mut out,
            "Working Files",
            format!(
                "{} ({} bytes)",
                self.working_files, self.working_files_bytes
            ),
        );
        line(
            &mut out,
            "Archive moves",
            self.archive_moves.len().to_string(),
        );
        line(
            &mut out,
            "Files created / changed",
            format!("{} / {}", self.files_created, self.files_changed),
        );
        line(&mut out, "Errors", self.errors.len().to_string());
        let list = |out: &mut String, heading: &str, items: &[Item]| {
            if items.is_empty() {
                return;
            }
            out.push_str(&format!(
                "\n## {heading}\n\n| Path | Title | Detail |\n|---|---|---|\n"
            ));
            for item in items {
                let cell = |s: &str| s.replace('|', "\\|").replace('\n', " ");
                out.push_str(&format!(
                    "| {} | {} | {} |\n",
                    cell(&item.path),
                    cell(&item.title),
                    cell(&item.detail)
                ));
            }
        };
        list(
            &mut out,
            "Open tasks that need an issue",
            &self.open_need_issue,
        );
        list(
            &mut out,
            "Mapping rows not applied",
            &self.mapping_not_applied,
        );
        list(
            &mut out,
            "Open tasks matched without a mapping row",
            &self.open_matched,
        );
        list(&mut out, "Split files", &self.split_files);
        list(
            &mut out,
            "Tracked projects and their workspaces",
            &self.projects_bound,
        );
        list(
            &mut out,
            "Tracker projects without a workspace",
            &self.tracker_projects_without_workspace,
        );
        list(
            &mut out,
            "Working Files skipped",
            &self.working_files_skipped,
        );
        if !self.archive_moves.is_empty() {
            out.push_str("\n## Archive moves\n\n");
            for path in &self.archive_moves {
                out.push_str(&format!("- `{path}`\n"));
            }
        }
        if !self.errors.is_empty() {
            out.push_str("\n## Errors\n\n");
            for error in &self.errors {
                out.push_str(&format!("- {error}\n"));
            }
        }
        out
    }

    /// `report.json` and `report.md` in `dir`, each scrubbed whole.
    pub(crate) fn write(&self, dir: &Path, secrets: &[&str]) -> Result<(), String> {
        std::fs::create_dir_all(dir)
            .map_err(|_| "The report folder couldn't be created.".to_string())?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|_| "The report couldn't be encoded.".to_string())?;
        files::write_out(dir, "report.json", scrub(&json, secrets).as_bytes())?;
        files::write_out(
            dir,
            "report.md",
            scrub(&self.markdown(), secrets).as_bytes(),
        )
    }
}

/// A folder the run writes into must sit outside the vault.
/// Checked on the canonical nearest existing parent before anything is
/// created, so a missing folder is never made inside the vault.
pub(crate) fn outside_vault(vault: &Path, dir: &Path, flag: &str) -> Result<(), String> {
    let vault = vault
        .canonicalize()
        .map_err(|_| "The vault folder doesn't exist.".to_string())?;
    let absolute = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| format!("{flag} couldn't be resolved."))?
            .join(dir)
    };
    let mut existing = absolute.as_path();
    while std::fs::symlink_metadata(existing).is_err() {
        existing = existing
            .parent()
            .ok_or_else(|| format!("{flag} couldn't be resolved."))?;
    }
    let rest = absolute
        .strip_prefix(existing)
        .map_err(|_| format!("{flag} couldn't be resolved."))?;
    if rest
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(format!(
            "{flag} must not contain .. after its existing part."
        ));
    }
    let resolved = existing
        .canonicalize()
        .map_err(|_| format!("{flag} couldn't be resolved."))?
        .join(rest);
    if resolved.starts_with(&vault) || vault.starts_with(&resolved) {
        return Err(format!("{flag} must be outside the vault."));
    }
    std::fs::create_dir_all(&resolved).map_err(|_| format!("{flag} couldn't be created."))
}

fn notion_client(settings: &Settings) -> Result<notion::NotionClient, String> {
    let token = settings
        .notion_token
        .as_deref()
        .ok_or("No Notion token in the config (notion.token or notionToken).")?;
    notion::NotionClient::new(token, &settings.notion_version)
}

/// Everything a run needs from outside the vault, so tests can hand in
/// their stubs.
pub(crate) struct Remotes<'a> {
    pub notion: Option<&'a notion::NotionClient>,
    pub tracker: Option<&'a dyn TrackerAdapter>,
    pub file_cap: u64,
    pub secrets: Vec<String>,
}

/// One mode, run against `args.vault`: the summary line, or the error,
/// with every secret scrubbed.
/// `--apply` and `--rekey` refuse while config.json still has
/// `notion.twoWaySync` on: Hollow no longer runs Notion two-way sync, but a
/// build from before its removal (a rollback, or a second build on the same
/// vault) would write the same task files.
pub(crate) const TWO_WAY_BLOCK: &str = "Notion two-way sync is on in config.json (notion.twoWaySync). Turn it off and restart Hollow before --apply or --rekey: a Hollow build from before its removal would write the same task files.";

pub(crate) async fn run_with(
    args: &Args,
    settings: &Settings,
    remotes: &Remotes<'_>,
) -> Result<String, String> {
    let secrets: Vec<&str> = remotes.secrets.iter().map(String::as_str).collect();
    run_mode(args, settings, remotes, &secrets)
        .await
        .map(|s| scrub(&s, &secrets))
        .map_err(|e| scrub(&e, &secrets))
}

async fn run_mode(
    args: &Args,
    settings: &Settings,
    remotes: &Remotes<'_>,
    secrets: &[&str],
) -> Result<String, String> {
    let vault = &args.vault;
    if !vault.join("tasks").is_dir() && !vault.join("workspaces").is_dir() {
        return Err("The vault has no tasks or workspaces folder.".into());
    }
    let need_notion = || {
        remotes
            .notion
            .ok_or("No Notion token in the config (notion.token or notionToken).".to_string())
    };
    let need_tracker = || remotes.tracker.ok_or("No tracker adapter.".to_string());
    if !matches!(args.mode, Mode::Rollback(_)) {
        settings.check_scope()?;
    }
    match &args.mode {
        Mode::Preflight => {
            let staging = args.staging.as_ref().unwrap();
            outside_vault(vault, staging, "--staging")?;
            let lines = preflight(args, settings, remotes, staging).await;
            let text = scrub(&lines.join("\n"), secrets);
            files::write_out(staging, "preflight.txt", format!("{text}\n").as_bytes())?;
            Ok(text)
        }
        Mode::DryRun => {
            let staging = args.staging.as_ref().unwrap();
            outside_vault(vault, staging, "--staging")?;
            let mut report = Report {
                mode: "dry-run".into(),
                ..Report::default()
            };
            let mut overlay = files::Overlay::new(vault);
            let notion = need_notion()?;
            records::import(
                notion,
                settings,
                &mut overlay,
                remotes.file_cap,
                &mut report,
            )
            .await?;
            let tracker = need_tracker()?;
            let mapping = rekey::read_mapping(&args.mapping, &args.bind_projects)?;
            rekey::rekey(tracker, settings, &mapping, &mut overlay, &mut report).await?;
            overlay.stage(staging, &mut report)?;
            report.write(staging, secrets)?;
            Ok(format!(
                "Dry run staged {} new and {} changed files in {}; {} errors. Report: report.md",
                report.files_created,
                report.files_changed,
                staging.display(),
                report.errors.len()
            ))
        }
        Mode::Apply | Mode::Rekey => {
            if settings.two_way_sync {
                return Err(TWO_WAY_BLOCK.into());
            }
            let backup = args.backup_dir.as_ref().unwrap();
            outside_vault(vault, backup, "--backup-dir")?;
            let mut report = Report {
                mode: args.mode.name().into(),
                ..Report::default()
            };
            let mut overlay = files::Overlay::new(vault);
            if args.mode == Mode::Apply {
                records::import(
                    need_notion()?,
                    settings,
                    &mut overlay,
                    remotes.file_cap,
                    &mut report,
                )
                .await?;
            } else {
                let mapping = rekey::read_mapping(&args.mapping, &args.bind_projects)?;
                rekey::rekey(
                    need_tracker()?,
                    settings,
                    &mapping,
                    &mut overlay,
                    &mut report,
                )
                .await?;
            }
            let outcome = files::apply(vault, backup, args.mode.name(), &overlay, &mut report)?;
            let dir = outcome
                .manifest
                .as_ref()
                .and_then(|m| m.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| backup.join(format!("{}-unchanged", args.mode.name())));
            report.write(&dir, secrets)?;
            Ok(match outcome.manifest {
                Some(manifest) => format!(
                    "{}: {} created, {} changed, {} moved. Manifest: {}",
                    args.mode.name(),
                    report.files_created,
                    report.files_changed,
                    outcome.moved,
                    manifest.display()
                ),
                None => format!("{}: nothing to change.", args.mode.name()),
            })
        }
        Mode::Rollback(manifest) => files::rollback(vault, manifest),
    }
}

async fn preflight(
    args: &Args,
    settings: &Settings,
    remotes: &Remotes<'_>,
    staging: &Path,
) -> Vec<String> {
    let mut lines = vec![format!(
        "Notion two-way sync: {}",
        if settings.two_way_sync {
            "ON (turn it off before --apply or --rekey)"
        } else {
            "off"
        }
    )];
    lines.push(format!(
        "Notion token: {}",
        if settings.notion_token.is_some() {
            "present"
        } else {
            "missing"
        }
    ));
    lines.push(format!(
        "Tracker key: {}",
        if remotes.tracker.is_some() {
            "present"
        } else {
            "missing"
        }
    ));
    if settings.tracker_assumed {
        lines.push(format!(
            "Tracker settings: none in the config; assumed team {DEFAULT_TEAM_KEY} ({DEFAULT_TEAM_ID})"
        ));
    }
    if let Some(tracker) = remotes.tracker {
        match tracker.whoami().await {
            Ok(user) => lines.push(format!("Tracker whoami: {}", user.name)),
            Err(e) => lines.push(format!("Tracker whoami failed: {e}")),
        }
        match tracker.scope().await {
            Ok(team) => lines.push(format!(
                "Tracker team: {} ({}, {})",
                team.name, team.key, team.id
            )),
            Err(e) => lines.push(format!("Tracker team failed: {e}")),
        }
        match tracker.members().await {
            Ok(members) => {
                // Name, display name or email (Sam's tracker name is his
                // email address).
                let found = members.iter().any(|m| m.answers_to(&args.member));
                lines.push(format!(
                    "{}: {}",
                    args.member,
                    if found { "member" } else { "not a member" }
                ));
            }
            Err(e) => lines.push(format!("Team members failed: {e}")),
        }
    }
    // Seconds and a counter keep a repeated preflight from colliding with
    // an earlier snapshot (snapshots only go into a new folder).
    let base = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut stamp = base.clone();
    let mut n = 1;
    while staging.join(format!("vault-snapshot-{stamp}")).exists() {
        n += 1;
        stamp = format!("{base}-{n}");
    }
    match files::snapshot(
        &args.vault,
        &staging.join(format!("vault-snapshot-{stamp}")),
    ) {
        Ok(count) => lines.push(format!(
            "Snapshot: {count} files copied to vault-snapshot-{stamp}"
        )),
        Err(e) => lines.push(format!("Snapshot failed: {e}")),
    }
    lines
}

/// `hollow tracker-import …`: the exit code.
pub fn main(argv: Vec<String>) -> i32 {
    let args = match parse_args(&argv) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("tracker-import: {e}\n{USAGE}");
            return 2;
        }
    };
    let settings = match Settings::load(&args.config) {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("tracker-import: {e}");
            return 2;
        }
    };
    let notion = match args.mode {
        Mode::Rekey | Mode::Rollback(_) => None,
        _ => notion_client(&settings).ok(),
    };
    let key = match args.mode {
        Mode::Apply | Mode::Rollback(_) => None,
        _ => tracker_key(settings.provider()),
    };
    let tracker = key
        .as_deref()
        .and_then(|key| tracker::adapter_with_key(&settings.tracker, key).ok());
    let mut secrets: Vec<String> = settings.notion_token.iter().cloned().collect();
    secrets.extend(key);
    let remotes = Remotes {
        notion: notion.as_ref(),
        tracker: tracker.as_deref(),
        file_cap: FILE_CAP,
        secrets,
    };
    let secret_refs: Vec<&str> = remotes.secrets.iter().map(String::as_str).collect();
    match tauri::async_runtime::block_on(run_with(&args, &settings, &remotes)) {
        Ok(summary) => {
            println!("{}", scrub(&summary, &secret_refs));
            0
        }
        Err(e) => {
            eprintln!("tracker-import: {}", scrub(&e, &secret_refs));
            1
        }
    }
}
