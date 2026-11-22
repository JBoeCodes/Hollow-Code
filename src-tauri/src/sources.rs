//! Sources of truth (overhaul H4a; design `sources-and-import-2026-11-13`
//! section 3, contract `docs/TRACKER_SYNC.md#sources-of-truth-h4a`).
//!
//! One rule answers "who owns this field of this item": [`resolve`] reads
//! where a workspace's tasks live, [`owner_of`] says who owns one field of
//! one item, and [`label`] is the words every surface shows ("Tracked in
//! Linear", "Stored in Hollow", "Read-only from Notion"), computed here and
//! sent as data so the phone and the desktop never disagree.
//!
//! **Config** (`config.json`, top-level `sources`, beside `tracker`):
//!
//! ```json
//! "sources": {
//!   "version": 1,
//!   "tasks": { "owner": "tracker" },
//!   "knowledge": { "owner": "vault", "references": [ { "connector": "notion-official", "label": "Notion archive" } ] }
//! }
//! ```
//!
//! `tasks.owner` is `vault` or `tracker`; the provider and its teams come
//! only from the `tracker` block, so one place names a team. A missing block
//! (or a missing `tasks`) reads as `tracker` when `tracker.provider` is set
//! and `vault` otherwise: today's config means what it meant. Knowledge is
//! always the vault's (N27); `references` name connector-library connectors
//! agents may read, read-only, and never hold a key. Personal context has no
//! setting. Loading is tolerant like `tracker::settings`: a bad value is
//! ignored with a line in `problems`.
//!
//! **Workspace override** (`PROJECT.md`, else `WORKSPACE.md`, frontmatter):
//! `tasks_source: vault` or `tasks_source: tracker` (with the existing
//! `tracker`, `tracker_project` and `tracker_team` keys); no key inherits.
//! An override can never reach a provider, account or team the `tracker`
//! block doesn't allow (`ProviderInfo::allows` stays the one guard): one
//! that tries is refused with a problem and the workspace inherits Settings.
//!
//! Changing an owner is a migration ([`migration`]), never a Save.

use crate::frontmatter;
use crate::tracker::settings::{self as tracker_settings, KeyState, TrackerConfig};
use crate::tracker::{Field, IDENTITY_KEYS, READ_ONLY_KEYS};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) mod migration;
#[cfg(test)]
mod tests;

/// The `sources` block version this build reads and writes.
pub const VERSION: u32 = 1;
/// The workspace override key.
pub const OVERRIDE_KEY: &str = "tasks_source";
/// The end of the "Was tracked in ..." line a switch to Hollow adds to a
/// tracked workspace's PROJECT.md.
pub const WAS_TRACKED_MARK: &str = "<!-- hollow:was-tracked -->";

/// Task-file keys the mirror writes beyond the fields, read-only and
/// identity keys (`tracker::mirror::task`).
const TASK_MIRROR_KEYS: &[&str] = &["status_type", "scope", "tracker_pending", "project_url"];
/// The project fields Hollow sends (the engine's `owned_field("project")`).
pub(crate) const PROJECT_FIELDS: &[&str] = &[
    "title",
    "description",
    "status",
    "lead",
    "start_date",
    "target_date",
];
/// Project-file keys the mirror writes and never sends.
const PROJECT_MIRROR_KEYS: &[&str] = &[
    "status_type",
    "scope",
    "tracker",
    "tracker_project",
    "tracker_state",
    "tracker_url",
    "tracker_team",
    "tracker_pending",
];

/// Who a role or a workspace's tasks belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesOwnerKind"))]
#[serde(rename_all = "camelCase")]
pub enum OwnerKind {
    Vault,
    Tracker,
}

impl OwnerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OwnerKind::Vault => "vault",
            OwnerKind::Tracker => "tracker",
        }
    }

    pub fn parse(text: &str) -> Option<OwnerKind> {
        match text.trim() {
            "vault" => Some(OwnerKind::Vault),
            "tracker" => Some(OwnerKind::Tracker),
            _ => None,
        }
    }
}

/// The three roles of design section 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesRole"))]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Tasks,
    Knowledge,
    Personal,
}

/// Who owns one field ([`owner_of`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    /// The provider id (`linear`).
    Tracker(String),
    Vault,
    /// A connector read for knowledge, never written.
    ReadOnly(String),
}

/// What kind of item a field belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Task,
    Project,
}

/// A connector agents may read for knowledge. Read-only; its secrets stay
/// with the connector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesReference"))]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub connector: String,
    #[serde(default)]
    pub label: Option<String>,
}

/// The `sources` block as loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcesConfig {
    /// config.json has a `sources` block.
    pub present: bool,
    /// Where tasks and projects live.
    pub tasks: OwnerKind,
    /// `tasks.owner` was written (false: read from the tracker block).
    pub tasks_set: bool,
    pub references: Vec<Reference>,
    /// Values ignored while loading, for Settings to show.
    pub problems: Vec<String>,
}

fn valid_connector(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl SourcesConfig {
    /// The derived reading: what a config without the block means.
    pub fn derived(tracker: &TrackerConfig) -> SourcesConfig {
        SourcesConfig {
            present: false,
            tasks: if tracker.provider.is_some() {
                OwnerKind::Tracker
            } else {
                OwnerKind::Vault
            },
            tasks_set: false,
            references: Vec::new(),
            problems: Vec::new(),
        }
    }

    /// Load the block tolerantly (never fails).
    pub fn from_value(value: Option<&Value>, tracker: &TrackerConfig) -> SourcesConfig {
        let mut config = SourcesConfig::derived(tracker);
        let Some(value) = value.filter(|v| !v.is_null()) else {
            return config;
        };
        config.present = true;
        let Some(block) = value.as_object() else {
            config.problems.push(
                "The sources settings aren't an object; tasks follow the tracker settings.".into(),
            );
            return config;
        };
        match block.get("version") {
            None | Some(Value::Null) => {}
            Some(v) if v.as_u64() == Some(VERSION as u64) => {}
            Some(_) => {
                config.problems.push(
                    "The sources settings are a version this Hollow doesn't read; tasks follow the tracker settings."
                        .into(),
                );
                return config;
            }
        }
        match block.get("tasks") {
            None | Some(Value::Null) => {}
            Some(Value::Object(tasks)) => {
                for key in tasks.keys().filter(|k| k.as_str() != "owner") {
                    let key = if valid_connector(key) { key } else { "a key" };
                    config.problems.push(format!(
                        "Tasks name their tracker and team in the tracker settings only; {key} was ignored."
                    ));
                }
                match tasks
                    .get("owner")
                    .and_then(Value::as_str)
                    .map(OwnerKind::parse)
                {
                    Some(Some(owner)) => {
                        config.tasks = owner;
                        config.tasks_set = true;
                    }
                    None if tasks.get("owner").is_none_or(Value::is_null) => {}
                    _ => config.problems.push(
                        "Tasks' owner isn't vault or tracker; tasks follow the tracker settings."
                            .into(),
                    ),
                }
            }
            Some(_) => config.problems.push(
                "The tasks setting isn't an object; tasks follow the tracker settings.".into(),
            ),
        }
        if config.tasks == OwnerKind::Tracker && tracker.provider.is_none() {
            config.problems.push(
                "Tasks come from the tracker, but no tracker is chosen in Settings, Sources."
                    .into(),
            );
        }
        match block.get("knowledge") {
            None | Some(Value::Null) => {}
            Some(Value::Object(knowledge)) => {
                if let Some(owner) = knowledge.get("owner").filter(|v| !v.is_null()) {
                    if owner.as_str() != Some("vault") {
                        config.problems.push(
                            "Knowledge always lives in your Hollow folder; its owner setting was ignored."
                                .into(),
                        );
                    }
                }
                config.references = references(knowledge.get("references"), &mut config.problems);
            }
            Some(_) => config
                .problems
                .push("The knowledge setting isn't an object; it was ignored.".into()),
        }
        if block.get("personal").is_some_and(|v| !v.is_null()) {
            config.problems.push(
                "Personal context always lives in your Hollow folder; the personal setting was ignored."
                    .into(),
            );
        }
        config
    }

    /// The `sources` and `tracker` blocks of a config file's map.
    pub fn from_map(map: &BTreeMap<String, Value>) -> (SourcesConfig, TrackerConfig) {
        let tracker = TrackerConfig::from_value(map.get("tracker").unwrap_or(&Value::Null));
        (
            SourcesConfig::from_value(map.get("sources"), &tracker),
            tracker,
        )
    }
}

fn references(value: Option<&Value>, problems: &mut Vec<String>) -> Vec<Reference> {
    let mut out: Vec<Reference> = Vec::new();
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return out;
    };
    let Some(items) = value.as_array() else {
        problems.push("Knowledge references aren't a list; they were ignored.".into());
        return out;
    };
    for item in items {
        let Some(item) = item.as_object() else {
            problems.push("A knowledge reference isn't an object; it was ignored.".into());
            continue;
        };
        let Some(connector) = item
            .get("connector")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|c| valid_connector(c))
        else {
            problems.push(
                "A knowledge reference doesn't name a connector by its id; it was ignored.".into(),
            );
            continue;
        };
        if item.keys().any(|k| k != "connector" && k != "label") {
            // Never echo the value: it may be a pasted secret.
            problems.push(format!(
                "The {connector} reference holds only a connector and a label; other values were ignored (secrets stay with the connector)."
            ));
        }
        if out.iter().any(|r| r.connector == connector) {
            problems.push(format!(
                "{connector} is listed twice; the second was ignored."
            ));
            continue;
        }
        let label = item
            .get("label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| l.chars().take(80).collect());
        out.push(Reference {
            connector: connector.to_string(),
            label,
        });
    }
    out
}

/// Write `tasks` and/or `references` into the config file's `sources` block
/// (created when missing; other keys of the block kept).
pub fn write_block(
    map: &mut BTreeMap<String, Value>,
    tasks: Option<OwnerKind>,
    references: Option<&[Reference]>,
) {
    let block = map
        .entry("sources".to_string())
        .or_insert_with(|| json!({}));
    if !block.is_object() {
        *block = json!({});
    }
    let block = block.as_object_mut().expect("object assigned above");
    block.insert("version".into(), json!(VERSION));
    if let Some(owner) = tasks {
        block.insert("tasks".into(), json!({ "owner": owner.as_str() }));
    }
    if let Some(references) = references {
        let refs: Vec<Value> = references
            .iter()
            .map(|r| match &r.label {
                Some(label) => json!({ "connector": r.connector, "label": label }),
                None => json!({ "connector": r.connector }),
            })
            .collect();
        block.insert(
            "knowledge".into(),
            json!({ "owner": "vault", "references": refs }),
        );
    }
}

/// What Hollow knows about reaching the tracker, gathered by the caller (the
/// saved key's state and the ledger), so the rules here touch no Keychain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    pub key: KeyState,
    /// The last successful pass.
    pub last_read: Option<String>,
    /// Consecutive failed passes (0: the last one worked).
    pub failures: u32,
}

impl Default for Access {
    fn default() -> Self {
        Access {
            key: KeyState::Saved,
            last_read: None,
            failures: 0,
        }
    }
}

/// Settings as one value: the two blocks and how reachable the tracker is.
#[derive(Debug, Clone)]
pub struct Context {
    pub sources: SourcesConfig,
    pub tracker: TrackerConfig,
    pub access: Access,
}

impl Context {
    pub fn from_map(map: &BTreeMap<String, Value>, access: Access) -> Context {
        let (sources, tracker) = SourcesConfig::from_map(map);
        Context {
            sources,
            tracker,
            access,
        }
    }
}

/// A workspace's own reading of where its tasks live.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Override {
    /// `tasks_source`; None inherits Settings.
    pub choice: Option<OwnerKind>,
    pub tracker: Option<String>,
    pub tracker_project: Option<String>,
    pub tracker_team: Option<String>,
    pub tracker_url: Option<String>,
    /// The "Was tracked in ..." line's link, when a switch added one.
    pub was_tracked: Option<String>,
    /// The workspace title, for messages.
    pub title: Option<String>,
    /// The file `tasks_source` lives in (or would): PROJECT.md when it exists.
    pub file: String,
    pub problems: Vec<String>,
}

fn fm_text(fm: &frontmatter::Frontmatter, key: &str) -> Option<String> {
    fm.get(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The link on a "Was tracked in ..." line.
pub(crate) fn was_tracked_link(text: &str) -> Option<String> {
    let line = text.lines().find(|l| l.contains(WAS_TRACKED_MARK))?;
    let start = line.find("](")? + 2;
    let end = start + line[start..].find(')')?;
    Some(line[start..end].to_string())
}

/// Read a workspace's override from its PROJECT.md, then WORKSPACE.md.
pub fn read_override(vault: &Path, workspace: &str) -> Result<Override, String> {
    let dir = crate::workspaces::folder(vault, workspace)?;
    let mut out = Override {
        file: format!("workspaces/{workspace}/WORKSPACE.md"),
        ..Override::default()
    };
    let mut found = false;
    for name in ["PROJECT.md", "WORKSPACE.md"] {
        let path = dir.join(name);
        let Ok((text, _)) = crate::workspaces::read_prefix(&path, 64 * 1024) else {
            continue;
        };
        if name == "PROJECT.md" {
            out.file = format!("workspaces/{workspace}/PROJECT.md");
        }
        let fm = frontmatter::parse(&text);
        out.title = out.title.or_else(|| fm_text(&fm, "title"));
        out.tracker = out.tracker.or_else(|| fm_text(&fm, "tracker"));
        out.tracker_project = out
            .tracker_project
            .or_else(|| fm_text(&fm, "tracker_project"));
        out.tracker_team = out.tracker_team.or_else(|| fm_text(&fm, "tracker_team"));
        out.tracker_url = out.tracker_url.or_else(|| fm_text(&fm, "tracker_url"));
        out.was_tracked = out.was_tracked.or_else(|| was_tracked_link(&text));
        if found {
            continue;
        }
        if let Some(raw) = fm_text(&fm, OVERRIDE_KEY) {
            found = true;
            match OwnerKind::parse(&raw) {
                Some(choice) => out.choice = Some(choice),
                None => out.problems.push(format!(
                    "{name}'s tasks_source isn't vault or tracker; this workspace inherits Settings."
                )),
            }
        }
    }
    if out.tracker.as_deref() == Some("none") {
        out.tracker = None;
    }
    Ok(out)
}

/// Where one workspace's (or the global) tasks live, and how to say it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesBinding"))]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub role: Role,
    pub owner: OwnerKind,
    /// The workspace has no override and follows Settings.
    pub inherited: bool,
    /// The owner was chosen (a `sources` setting or an override), not read
    /// from whether a tracker is set up.
    pub chosen: bool,
    pub workspace: Option<String>,
    /// The provider id (`linear`) when the owner is a tracker.
    pub provider: Option<String>,
    /// "Linear", or a reference's label.
    pub provider_name: Option<String>,
    pub team_id: Option<String>,
    pub team_key: Option<String>,
    pub tracker_project: Option<String>,
    /// The link a switch to Hollow kept ("Was tracked in Linear").
    pub was_tracked: Option<String>,
    /// A knowledge reference's connector: everything it holds is read-only.
    pub read_only: Option<String>,
    /// The tracker answered on the last pass and the key is saved.
    pub reachable: bool,
    /// The last successful read while unreachable.
    pub unreachable_since: Option<String>,
    pub problems: Vec<String>,
}

impl Binding {
    fn vault(role: Role) -> Binding {
        Binding {
            role,
            owner: OwnerKind::Vault,
            inherited: false,
            chosen: true,
            workspace: None,
            provider: None,
            provider_name: None,
            team_id: None,
            team_key: None,
            tracker_project: None,
            was_tracked: None,
            read_only: None,
            reachable: true,
            unreachable_since: None,
            problems: Vec::new(),
        }
    }

    /// A knowledge reference: read-only from its connector.
    pub fn reference(reference: &Reference, name: Option<&str>) -> Binding {
        Binding {
            provider_name: Some(
                reference
                    .label
                    .clone()
                    .or_else(|| name.map(str::to_string))
                    .unwrap_or_else(|| reference.connector.clone()),
            ),
            read_only: Some(reference.connector.clone()),
            ..Binding::vault(Role::Knowledge)
        }
    }
}

/// The team a workspace names (`tracker_team`: a key or an id), when it is
/// one of the tracker block's teams and the provider's guard allows it.
fn allowed_team(tracker: &TrackerConfig, named: &str) -> Option<(String, Option<String>)> {
    let info = tracker.provider_info()?;
    let named = named.trim();
    let pairs = [
        (tracker.team_id.as_ref(), tracker.team_key.as_ref()),
        (
            tracker.personal_team_id.as_ref(),
            tracker.personal_team_key.as_ref(),
        ),
    ];
    pairs.into_iter().find_map(|(id, key)| {
        let id = id?;
        let matches = id == named || key.is_some_and(|k| k.eq_ignore_ascii_case(named));
        (matches && info.allows(id)).then(|| (id.clone(), key.cloned()))
    })
}

/// The role rows' task binding for the whole vault (Settings' own row).
pub fn global_tasks(ctx: &Context) -> Binding {
    let mut binding = Binding::vault(Role::Tasks);
    binding.chosen = ctx.sources.tasks_set;
    binding.problems = ctx.sources.problems.clone();
    if ctx.sources.tasks == OwnerKind::Tracker {
        tracker_binding(ctx, &mut binding, None);
    }
    binding
}

fn tracker_binding(ctx: &Context, binding: &mut Binding, team: Option<(String, Option<String>)>) {
    binding.owner = OwnerKind::Tracker;
    let info = ctx.tracker.provider_info();
    binding.provider = ctx.tracker.provider.clone();
    binding.provider_name = info.map(|p| p.display_name.to_string());
    let (team_id, team_key) = team.unwrap_or_else(|| {
        (
            ctx.tracker.team_id.clone().unwrap_or_default(),
            ctx.tracker.team_key.clone(),
        )
    });
    binding.team_id = Some(team_id).filter(|t| !t.is_empty());
    binding.team_key = team_key;
    let set_up = info.is_some() && binding.team_id.is_some();
    binding.reachable = set_up && ctx.access.key == KeyState::Saved && ctx.access.failures == 0;
    if !binding.reachable {
        binding.unreachable_since = ctx.access.last_read.clone();
    }
}

/// Where `workspace`'s tasks live (`None`: Settings' own reading).
pub fn resolve(ctx: &Context, vault: &Path, workspace: Option<&str>) -> Binding {
    let global = global_tasks(ctx);
    let Some(id) = workspace else {
        return global;
    };
    let inherit = |problems: Vec<String>, ov: Option<&Override>| {
        let mut b = global.clone();
        b.inherited = true;
        b.workspace = Some(id.to_string());
        b.problems = problems;
        if let Some(ov) = ov {
            b.tracker_project = ov.tracker_project.clone();
            b.was_tracked = ov.was_tracked.clone();
        }
        b
    };
    let ov = match read_override(vault, id) {
        Ok(ov) => ov,
        Err(error) => return inherit(vec![error], None),
    };
    let mut problems = ov.problems.clone();
    // The workspace's own tracker identity may name only what Settings allows.
    let named_provider = ov
        .tracker
        .as_deref()
        .filter(|p| Some(*p) != ctx.tracker.provider.as_deref());
    let team = match ov.tracker_team.as_deref() {
        Some(named) => match allowed_team(&ctx.tracker, named) {
            Some(team) => Some(team),
            None => {
                problems.push(format!(
                    "This workspace names team {} that Settings doesn't allow; it can't widen Hollow's scope.",
                    if tracker_settings::looks_like_team_key(named) { named } else { "a team" }
                ));
                None
            }
        },
        None => None,
    };
    match ov.choice {
        None => {
            let mut b = inherit(problems, Some(&ov));
            if b.owner == OwnerKind::Tracker {
                if let Some(team) = team.filter(|_| named_provider.is_none()) {
                    b.team_id = Some(team.0);
                    b.team_key = team.1;
                }
            }
            b
        }
        Some(OwnerKind::Vault) => {
            let mut b = Binding::vault(Role::Tasks);
            b.workspace = Some(id.to_string());
            b.tracker_project = ov.tracker_project.clone();
            b.was_tracked = ov.was_tracked.clone();
            b.problems = problems;
            b
        }
        Some(OwnerKind::Tracker) => {
            let refused = if ctx.tracker.provider_info().is_none() || ctx.tracker.team_id.is_none()
            {
                Some("This workspace asks for the tracker, but Settings has no tracker and team; it inherits Settings.".to_string())
            } else if let Some(p) = named_provider {
                let p = if valid_connector(p) {
                    p
                } else {
                    "another tracker"
                };
                Some(format!(
                    "This workspace names {p}, which Settings doesn't use; it can't widen Hollow's scope and inherits Settings."
                ))
            } else if ov.tracker_team.is_some() && team.is_none() {
                Some("This workspace's team isn't allowed; it inherits Settings.".to_string())
            } else {
                None
            };
            if let Some(why) = refused {
                problems.push(why);
                return inherit(problems, Some(&ov));
            }
            let mut b = Binding::vault(Role::Tasks);
            b.workspace = Some(id.to_string());
            b.tracker_project = ov.tracker_project.clone();
            b.was_tracked = ov.was_tracked.clone();
            b.problems = problems;
            tracker_binding(ctx, &mut b, team);
            b
        }
    }
}

/// Knowledge: always the vault's (N27).
pub fn knowledge() -> Binding {
    let mut b = Binding::vault(Role::Knowledge);
    b.chosen = false;
    b
}

/// Whether `key` of a `kind` item is one the tracker mirrors (writable or
/// read-only) for `provider`.
fn tracker_key(provider: &str, kind: ItemKind, key: &str) -> bool {
    match kind {
        ItemKind::Task => {
            crate::tracker::declared_writable_fields(provider)
                .iter()
                .any(|f| f.key() == key)
                || Field::from_key(key).is_some()
                || READ_ONLY_KEYS.contains(&key)
                || IDENTITY_KEYS.contains(&key)
                || TASK_MIRROR_KEYS.contains(&key)
        }
        ItemKind::Project => PROJECT_FIELDS.contains(&key) || PROJECT_MIRROR_KEYS.contains(&key),
    }
}

/// Who owns `key` of a `kind` item under `binding`: the tracker for the
/// provider's fields and the keys it mirrors read-only, the vault for every
/// other key (notes, evidence, local scheduling, claims), and a reference's
/// connector for everything a reference holds. A tracked binding whose
/// tracker is unreachable still answers the tracker (lost access keeps the
/// mirror; it never becomes the vault's).
pub fn owner_of(binding: &Binding, kind: ItemKind, key: &str) -> Owner {
    if let Some(connector) = &binding.read_only {
        return Owner::ReadOnly(connector.clone());
    }
    if binding.role != Role::Tasks || binding.owner == OwnerKind::Vault {
        return Owner::Vault;
    }
    let provider = binding.provider.clone().unwrap_or_default();
    if tracker_key(&provider, kind, key) {
        Owner::Tracker(provider)
    } else {
        Owner::Vault
    }
}

/// Whether Hollow sends `key` of a `kind` item to the tracker (the
/// provider's writable fields; the engine's field maps agree, see tests).
pub fn sends(binding: &Binding, kind: ItemKind, key: &str) -> bool {
    if binding.read_only.is_some() || binding.owner != OwnerKind::Tracker {
        return false;
    }
    let provider = binding.provider.clone().unwrap_or_default();
    match kind {
        ItemKind::Task => crate::tracker::declared_writable_fields(&provider)
            .iter()
            .any(|f| f.key() == key),
        ItemKind::Project => PROJECT_FIELDS.contains(&key),
    }
}

/// The facts [`label`] reads from one item's frontmatter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemFacts {
    pub tracker: Option<String>,
    pub tracker_id: Option<String>,
    pub tracker_project: Option<String>,
    pub tracker_url: Option<String>,
}

impl ItemFacts {
    /// An item file's facts (the Tasks list's labels, H4b, read them).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_text(text: &str) -> ItemFacts {
        let fm = frontmatter::parse(text);
        ItemFacts {
            tracker: fm_text(&fm, "tracker").filter(|t| t != "none"),
            tracker_id: fm_text(&fm, "tracker_id"),
            tracker_project: fm_text(&fm, "tracker_project"),
            tracker_url: fm_text(&fm, "tracker_url"),
        }
    }

    /// A tracker mirror (an issue or a project).
    pub fn mirrored(&self) -> bool {
        self.tracker.is_some() && (self.tracker_id.is_some() || self.tracker_project.is_some())
    }
}

/// How a label reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourceLabelKind"))]
#[serde(rename_all = "camelCase")]
pub enum LabelKind {
    Tracked,
    Stored,
    ReadOnly,
    Unreachable,
}

/// The words on an item, computed here and sent as data.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SourceLabel {
    pub kind: LabelKind,
    /// "Tracked in Linear", "Stored in Hollow", "Read-only from Notion",
    /// "Linear unreachable since 9:40 AM".
    pub text: String,
    pub provider_name: Option<String>,
    /// The item in its tracker, when there is one.
    pub url: Option<String>,
    /// "Open in Linear".
    pub open_label: Option<String>,
    /// The last successful read, while unreachable (RFC 3339).
    pub since: Option<String>,
    /// "Was tracked in Linear" and its link, after a switch to Hollow.
    pub was_tracked: Option<String>,
    pub was_tracked_url: Option<String>,
}

fn display_of(provider: Option<&str>) -> String {
    provider
        .and_then(tracker_settings::provider)
        .map(|p| p.display_name.to_string())
        .unwrap_or_else(|| "The tracker".into())
}

fn short_time(at: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(at) {
        Ok(at) => {
            let local = at.with_timezone(&chrono::Local);
            if local.date_naive() == chrono::Local::now().date_naive() {
                local.format("%-I:%M %p").to_string()
            } else {
                local.format("%b %-d, %-I:%M %p").to_string()
            }
        }
        Err(_) => at.to_string(),
    }
}

/// The label for `item` under `binding`.
pub fn label(binding: &Binding, item: &ItemFacts) -> SourceLabel {
    let mut out = SourceLabel {
        kind: LabelKind::Stored,
        text: "Stored in Hollow".into(),
        provider_name: None,
        url: None,
        open_label: None,
        since: None,
        was_tracked: None,
        was_tracked_url: None,
    };
    if binding.read_only.is_some() {
        let name = binding.provider_name.clone().unwrap_or_default();
        out.kind = LabelKind::ReadOnly;
        out.text = format!("Read-only from {name}");
        out.provider_name = Some(name);
        return out;
    }
    if binding.owner == OwnerKind::Tracker {
        let name = binding
            .provider_name
            .clone()
            .unwrap_or_else(|| display_of(item.tracker.as_deref()));
        out.provider_name = Some(name.clone());
        out.url = item.tracker_url.clone();
        if binding.reachable {
            out.kind = LabelKind::Tracked;
            out.text = format!("Tracked in {name}");
        } else {
            out.kind = LabelKind::Unreachable;
            out.since = binding.unreachable_since.clone();
            out.text = match &out.since {
                Some(at) => format!("{name} unreachable since {}", short_time(at)),
                None => format!("{name} unreachable"),
            };
        }
        if out.url.is_some() {
            out.open_label = Some(format!("Open in {name}"));
        }
        return out;
    }
    if item.mirrored() && !binding.chosen {
        // The tracker block went away under a mirror: lost access, never
        // "Stored in Hollow".
        let name = display_of(item.tracker.as_deref());
        out.kind = LabelKind::Unreachable;
        out.text = format!("{name} unreachable");
        out.provider_name = Some(name);
        out.url = item.tracker_url.clone();
        return out;
    }
    if item.mirrored() || binding.was_tracked.is_some() {
        let name = display_of(item.tracker.as_deref().or(binding.provider.as_deref()));
        out.was_tracked = Some(format!("Was tracked in {name}"));
        out.was_tracked_url = item
            .tracker_url
            .clone()
            .or_else(|| binding.was_tracked.clone());
    }
    out
}

/// What the ledger says, for status rows (no validation, no lock: display).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LedgerFacts {
    pub last_read: Option<String>,
    pub failures: u32,
    /// Outbox entries, unconfirmed creates and project creates.
    pub queued: u32,
    pub held: u32,
    pub conflicts: u32,
    pub generation: u32,
    pub stored_in_hollow: bool,
    pub mirrored_tasks: u32,
    pub mirrored_projects: u32,
}

fn count(n: usize) -> u32 {
    n.min(u32::MAX as usize) as u32
}

impl LedgerFacts {
    pub(crate) fn of(state: &crate::tracker_sync::Ledger) -> LedgerFacts {
        LedgerFacts {
            last_read: state.last_sync.clone(),
            failures: state.failures,
            queued: count(
                state.outbox.len()
                    + state.creates.values().filter(|c| c.issue.is_none()).count()
                    + state
                        .project_creates
                        .values()
                        .filter(|c| c.project.is_none() && c.error.is_none())
                        .count(),
            ),
            held: count(state.held.len()),
            conflicts: count(state.records.values().map(|r| r.conflicts.len()).sum()),
            generation: state.generation,
            stored_in_hollow: state.stored_in_hollow,
            mirrored_tasks: count(state.records.values().filter(|r| r.kind == "task").count()),
            mirrored_projects: count(
                state
                    .records
                    .values()
                    .filter(|r| r.kind == "project")
                    .count(),
            ),
        }
    }

    /// The ledger as saved, when it can be read.
    pub fn read(vault: &Path) -> Option<LedgerFacts> {
        read_ledger(vault).map(|state| LedgerFacts::of(&state))
    }
}

/// The sync ledger as saved, parsed but not validated (display reads).
pub(crate) fn read_ledger(vault: &Path) -> Option<crate::tracker_sync::Ledger> {
    let raw = std::fs::read(vault.join(crate::tracker_sync::STATE)).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// One role row of Settings, Sources.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesRoleRow"))]
#[serde(rename_all = "camelCase")]
pub struct RoleRow {
    pub role: Role,
    pub binding: Binding,
    pub label: SourceLabel,
    /// The last successful read of the tracker (tasks only).
    pub last_read: Option<String>,
    pub queued_edits: u32,
    /// Edits queued before an owner switch, held and never sent.
    pub held_edits: u32,
    pub conflicts: u32,
    pub problems: Vec<String>,
}

/// One workspace with an override, a tracker binding or a "Was tracked" line.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesWorkspaceRow"))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRow {
    pub id: String,
    pub title: Option<String>,
    pub binding: Binding,
    pub label: SourceLabel,
}

/// One knowledge reference and whether its connector is in the library.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesReferenceRow"))]
#[serde(rename_all = "camelCase")]
pub struct ReferenceRow {
    pub reference: Reference,
    pub name: Option<String>,
    pub in_library: bool,
    pub label: SourceLabel,
}

/// `sources_status`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SourcesStatus {
    /// Tasks and projects, Knowledge, Personal context, in that order.
    pub rows: Vec<RoleRow>,
    pub workspaces: Vec<WorkspaceRow>,
    pub references: Vec<ReferenceRow>,
    /// The ledger's owner generation.
    pub generation: u32,
    /// Tasks are stored in Hollow and the tracker pass reads and sends nothing.
    pub stored_in_hollow: bool,
    /// An owner switch that stopped part way; finish or roll it back.
    pub unfinished_migration: Option<String>,
    /// Task-file keys the tracker owns under Settings' binding (editors hold
    /// them), and the part Hollow sends; empty while tasks live in Hollow.
    pub tracker_task_keys: Vec<String>,
    pub sent_task_keys: Vec<String>,
    /// The same for a tracked workspace's PROJECT.md.
    pub tracker_project_keys: Vec<String>,
    pub sent_project_keys: Vec<String>,
    pub problems: Vec<String>,
}

/// The keys of a `kind` item [`owner_of`] gives the tracker, and the part
/// [`sends`] pushes.
fn tracker_keys(binding: &Binding, kind: ItemKind) -> (Vec<String>, Vec<String>) {
    let candidates: Vec<&str> = match kind {
        ItemKind::Task => Field::ALL
            .iter()
            .map(|f| f.key())
            .chain(READ_ONLY_KEYS.iter().copied())
            .chain(IDENTITY_KEYS.iter().copied())
            .chain(TASK_MIRROR_KEYS.iter().copied())
            .collect(),
        ItemKind::Project => PROJECT_FIELDS
            .iter()
            .chain(PROJECT_MIRROR_KEYS)
            .copied()
            .collect(),
    };
    let owned = candidates
        .iter()
        .filter(|key| matches!(owner_of(binding, kind, key), Owner::Tracker(_)))
        .map(|key| key.to_string())
        .collect();
    let sent = candidates
        .iter()
        .filter(|key| sends(binding, kind, key))
        .map(|key| key.to_string())
        .collect();
    (owned, sent)
}

/// Workspaces worth a row: an override, a tracker binding or a marker.
fn workspace_rows(ctx: &Context, vault: &Path) -> Vec<WorkspaceRow> {
    let Ok(entries) = std::fs::read_dir(vault.join("workspaces")) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|id| crate::workspaces::valid_id(id))
        .collect();
    ids.sort();
    ids.into_iter()
        .filter_map(|id| {
            let ov = read_override(vault, &id).ok()?;
            let shown = ov.choice.is_some()
                || ov.tracker_project.is_some()
                || ov.was_tracked.is_some()
                || !ov.problems.is_empty();
            if !shown {
                return None;
            }
            let binding = resolve(ctx, vault, Some(&id));
            let facts = ItemFacts {
                tracker: ov.tracker.clone(),
                tracker_id: None,
                tracker_project: ov.tracker_project.clone(),
                tracker_url: ov.tracker_url.clone(),
            };
            Some(WorkspaceRow {
                label: label(&binding, &facts),
                id,
                title: ov.title,
                binding,
            })
        })
        .collect()
}

/// Settings, Sources' rows (pure apart from reading the vault's workspace
/// files; the caller supplies the ledger facts and the connector library).
pub fn status(
    ctx: &Context,
    vault: &Path,
    ledger: Option<&LedgerFacts>,
    library: Option<&[(String, String)]>,
) -> SourcesStatus {
    let facts = ledger.cloned().unwrap_or_default();
    let tasks = global_tasks(ctx);
    let tasks_label = label(&tasks, &ItemFacts::default());
    let mut rows = vec![RoleRow {
        role: Role::Tasks,
        problems: tasks.problems.clone(),
        label: tasks_label,
        binding: tasks,
        last_read: facts.last_read.clone(),
        queued_edits: facts.queued,
        held_edits: facts.held,
        conflicts: facts.conflicts,
    }];
    let knowledge = knowledge();
    rows.push(RoleRow {
        role: Role::Knowledge,
        label: label(&knowledge, &ItemFacts::default()),
        binding: knowledge,
        last_read: None,
        queued_edits: 0,
        held_edits: 0,
        conflicts: 0,
        problems: Vec::new(),
    });
    let mut personal = Binding::vault(Role::Personal);
    personal.chosen = false;
    rows.push(RoleRow {
        role: Role::Personal,
        label: label(&personal, &ItemFacts::default()),
        binding: personal,
        last_read: None,
        queued_edits: 0,
        held_edits: 0,
        conflicts: 0,
        problems: Vec::new(),
    });
    let mut problems = ctx.sources.problems.clone();
    let references = ctx
        .sources
        .references
        .iter()
        .map(|reference| {
            let name = library.and_then(|lib| {
                lib.iter()
                    .find(|(id, _)| id == &reference.connector)
                    .map(|(_, name)| name.clone())
            });
            let in_library = library.is_none() || name.is_some();
            if !in_library {
                problems.push(format!(
                    "{} isn't in the connectors library; agents can't read it until it is added.",
                    reference.connector
                ));
            }
            let binding = Binding::reference(reference, name.as_deref());
            ReferenceRow {
                label: label(&binding, &ItemFacts::default()),
                reference: reference.clone(),
                name,
                in_library,
            }
        })
        .collect();
    if facts.stored_in_hollow && ctx.sources.tasks == OwnerKind::Tracker {
        problems.push(
            "The tracker pass is off: its history says tasks are stored in Hollow. Switch tasks to the tracker to sync again."
                .into(),
        );
    }
    let (tracker_task_keys, sent_task_keys) = tracker_keys(&rows[0].binding, ItemKind::Task);
    let (tracker_project_keys, sent_project_keys) =
        tracker_keys(&rows[0].binding, ItemKind::Project);
    SourcesStatus {
        tracker_task_keys,
        sent_task_keys,
        tracker_project_keys,
        sent_project_keys,
        rows,
        workspaces: workspace_rows(ctx, vault),
        references,
        generation: facts.generation,
        stored_in_hollow: facts.stored_in_hollow,
        unfinished_migration: None,
        problems,
    }
}

/// `sources_set`: what Save changes. Every field is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SourcesUpdate"))]
#[serde(rename_all = "camelCase")]
pub struct Update {
    /// Where tasks live for every workspace. A change is a switch: refused
    /// here, see `sources_migration_plan`.
    #[serde(default)]
    pub tasks_owner: Option<OwnerKind>,
    /// The whole knowledge reference list.
    #[serde(default)]
    pub references: Option<Vec<Reference>>,
}

/// Why Save refuses an owner change.
pub const SWITCH_NOT_SAVE: &str = "Changing where tasks live is a switch, not a Save: check it with sources_migration_plan (counts first), then run sources_migration_apply. Nothing was changed.";

/// Apply a Save to the config file's map, refusing before any change.
pub fn apply_update(map: &mut BTreeMap<String, Value>, update: &Update) -> Result<(), String> {
    let (sources, _) = SourcesConfig::from_map(map);
    if let Some(owner) = update.tasks_owner {
        if owner != sources.tasks {
            return Err(SWITCH_NOT_SAVE.into());
        }
    }
    let references = match &update.references {
        Some(list) => {
            let mut problems = Vec::new();
            let value = serde_json::to_value(list).map_err(|e| e.to_string())?;
            let clean = references(Some(&value), &mut problems);
            if let Some(problem) = problems.first() {
                return Err(format!("{problem} Nothing was changed."));
            }
            Some(clean)
        }
        None => None,
    };
    if update.tasks_owner.is_none() && references.is_none() {
        return Ok(());
    }
    write_block(map, update.tasks_owner, references.as_deref());
    Ok(())
}
