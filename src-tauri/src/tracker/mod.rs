//! The task tracker: provider-neutral types, the adapter trait and its
//! errors (overhaul A2; contract `docs/TRACKER_SYNC.md`).
//!
//! Hollow's projects and tasks live in a tracker (Linear, team ENG only, since
//! 2026-11-07); the vault mirrors them. Everything outside this module says
//! `tracker`: names, types, IPC, config, vault keys and copy. A provider's
//! name appears only inside its adapter file and in its display name, so a
//! second provider needs a new adapter file and nothing else.
//!
//! - [`settings`]: the tolerant `tracker` config block and the API key
//!   (Hollow's one Keychain bundle, plus the one-click copy from a
//!   provider's own Keychain item).
//! - [`ipc`]: the shapes the Settings panel and the Tasks page read.
//! - [`linear`]: the Linear adapter (`LinearAdapter`, the trait over
//!   Linear's GraphQL API with its scope guard, rate limits and paging).
//! - `stub`: a canned GraphQL server for tests (tests only).
//! - [`work`] and [`work_outbox`]: tracking delegated agent work (X1): the
//!   run's issue link, the facts the tracker gets and their crash-safe
//!   outbox in the sync ledger.
//! - [`decision_log`]: a decision record's build log comment (DR1b), through
//!   the same dry-run gate and outbox.
//! - [`status`]: the Friday status per tracked project (A18), written into
//!   Hollow, with an optional project status update in the tracker.
//!
//! Done/open logic reads a state's [`StateKind`], never its name, so a
//! provider's own words ("Delivered", "In Review") never matter.

pub(crate) mod chat_work;
pub(crate) mod commitment_work;
pub(crate) mod decision_log;
pub(crate) mod graphql;
pub mod ipc;
pub mod linear;
pub(crate) mod mirror;
pub mod settings;
pub(crate) mod status;
#[cfg(test)]
pub(crate) mod stub;
pub(crate) mod sync_fields;
pub(crate) mod waiting_work;
pub(crate) mod work;
pub(crate) mod work_outbox;

/// The provider registry is the one place the engine chooses an adapter.
pub(crate) fn adapter(
    config: &settings::TrackerConfig,
) -> Result<Box<dyn TrackerAdapter>, TrackerError> {
    match config.provider.as_deref() {
        Some(linear::PROVIDER) => Ok(Box::new(linear::LinearAdapter::from_config(config)?)),
        _ => Err(TrackerError::NotConfigured("Choose a task tracker.".into())),
    }
}

/// The adapter for one of the config's teams: the primary's own, or the
/// provider's scoped copy for the personal team (refused when the provider's
/// guard doesn't allow it).
pub(crate) fn adapter_for(
    config: &settings::TrackerConfig,
    team: &str,
) -> Result<Box<dyn TrackerAdapter>, TrackerError> {
    let primary = adapter(config)?;
    match primary.for_team(team) {
        None => Ok(primary),
        Some(other) => other,
    }
}

/// The same adapter with a key the caller read (the importer reads the
/// provider's own Keychain item without copying it into Hollow's bundle).
pub(crate) fn adapter_with_key(
    config: &settings::TrackerConfig,
    key: &str,
) -> Result<Box<dyn TrackerAdapter>, TrackerError> {
    match config.provider.as_deref() {
        Some(linear::PROVIDER) => Ok(Box::new(linear::LinearAdapter::from_config_with_key(
            config, key,
        )?)),
        _ => Err(TrackerError::NotConfigured("Choose a task tracker.".into())),
    }
}

/// No credential or network read. Unknown providers have no implied writes.
pub(crate) fn declared_writable_fields(provider: &str) -> &[Field] {
    match provider {
        linear::PROVIDER => linear::WRITABLE_FIELDS,
        _ => &[],
    }
}

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

/// A state's kind. Linear's state types; other providers map onto them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "TrackerStateKind"))]
#[serde(rename_all = "lowercase")]
pub enum StateKind {
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
}

impl StateKind {
    #[cfg(test)]
    pub const ALL: [StateKind; 5] = [
        StateKind::Backlog,
        StateKind::Unstarted,
        StateKind::Started,
        StateKind::Completed,
        StateKind::Canceled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            StateKind::Backlog => "backlog",
            StateKind::Unstarted => "unstarted",
            StateKind::Started => "started",
            StateKind::Completed => "completed",
            StateKind::Canceled => "canceled",
        }
    }

    /// A provider's type word, case-insensitively; `triage` reads as backlog
    /// and `cancelled` as canceled. Anything else is unknown (None).
    pub fn parse(word: &str) -> Option<StateKind> {
        match word.trim().to_ascii_lowercase().as_str() {
            "backlog" | "triage" => Some(StateKind::Backlog),
            "unstarted" => Some(StateKind::Unstarted),
            "started" => Some(StateKind::Started),
            "completed" => Some(StateKind::Completed),
            "canceled" | "cancelled" => Some(StateKind::Canceled),
            _ => None,
        }
    }

    /// Done for Hollow's purposes (hours saved, due lists): completed only.
    pub fn is_done(self) -> bool {
        self == StateKind::Completed
    }

    /// Closed: completed or canceled. Open lists leave these out.
    pub fn is_closed(self) -> bool {
        matches!(self, StateKind::Completed | StateKind::Canceled)
    }
}

/// One workflow state of the team.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerState {
    pub id: String,
    pub name: String,
    pub kind: StateKind,
    /// The provider's order within the kind (Linear's `position`).
    #[serde(default)]
    pub position: f64,
    #[serde(default)]
    pub color: Option<String>,
}

/// A person in the tracker. `email` is work data: never mirrored to the phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerUser {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    /// The provider's short display name, when it has one (Linear's
    /// `displayName`; team members only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub display_name: Option<String>,
}

impl TrackerUser {
    /// Whether `wanted` names this person: their name, display name or
    /// email, case-insensitively and with spaces collapsed, or an email's
    /// local part with dots and underscores read as spaces
    /// (`sam.taylor@...` answers to "Sam Taylor"). Some people's tracker
    /// name is their email address (T6b).
    pub fn answers_to(&self, wanted: &str) -> bool {
        let norm = |s: &str| {
            s.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        };
        let wanted = norm(wanted);
        if wanted.is_empty() {
            return false;
        }
        [
            Some(&self.name),
            self.display_name.as_ref(),
            self.email.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|known| {
            norm(known) == wanted
                || known
                    .split_once('@')
                    .is_some_and(|(local, _)| norm(&local.replace(['.', '_'], " ")) == wanted)
        })
    }
}

/// The one team Hollow may read and write. Every query names it; any node
/// from another team is dropped before it reaches the vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerTeam {
    pub id: String,
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerLabel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    /// The label group's name, when the label sits in one.
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerCycle {
    pub id: String,
    pub number: u32,
    #[serde(default)]
    pub name: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerMilestone {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub target_date: Option<String>,
    /// The project the milestone belongs to.
    #[serde(default)]
    pub project_id: Option<String>,
}

/// How one issue relates to another, read from the issue's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "TrackerRelationKind"))]
#[serde(rename_all = "camelCase")]
pub enum RelationKind {
    Blocks,
    BlockedBy,
    Related,
    DuplicateOf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerRelation {
    pub kind: RelationKind,
    /// The other issue's key ("ENG-201").
    pub key: String,
}

/// A link or attachment on an issue or project. Linear calls both
/// attachments; the adapter puts plain URL links in `links` and uploads or
/// integrations (a pull request, a file) in `attachments`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerLink {
    pub id: String,
    pub title: String,
    pub url: String,
    /// Where it came from (`url`, `github`, `upload`…), when known.
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// One comment. Read-only in Hollow (owned by the tracker); writing one from
/// the issue detail is its own row (B5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerComment {
    pub id: String,
    #[serde(default)]
    pub author: Option<String>,
    pub body: String,
    pub created_at: String,
    #[serde(default)]
    pub edited_at: Option<String>,
    /// The comment this one replies to.
    #[serde(default)]
    pub parent_id: Option<String>,
}

/// A comment that changed, with the issue it belongs to (the comments cursor
/// reports comment-only changes an issue's `updated_at` doesn't show).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerCommentChange {
    pub issue_id: String,
    pub team_id: String,
    /// When the comment last changed (RFC 3339): the comments cursor
    /// advances to the newest one after the last page.
    #[serde(default)]
    pub updated_at: String,
    pub comment: TrackerComment,
}

/// What one history entry changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "TrackerHistoryKind"))]
#[serde(rename_all = "camelCase")]
pub enum HistoryKind {
    State,
    Assignee,
    Priority,
    Labels,
    Project,
    Title,
    Description,
}

/// One change in an issue's history (the issue detail's activity), read
/// from the tracker on demand and never stored in the mirror. A provider
/// entry that changed several fields at once becomes one event per field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerHistoryEvent {
    /// The provider's entry id, with the field ("h1:state").
    pub id: String,
    /// When it changed (RFC 3339).
    pub at: String,
    /// Who changed it, when the provider says (an integration or an
    /// automation has no name).
    #[serde(default)]
    pub actor: Option<String>,
    pub kind: HistoryKind,
    /// The value before and after: a state's or person's or project's name,
    /// a priority (0 to 4) as text, a title. None for "nobody" or "none".
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    /// For a state change: the states' kinds, so the feed shows their icons.
    #[serde(default)]
    pub from_kind: Option<StateKind>,
    #[serde(default)]
    pub to_kind: Option<StateKind>,
    /// For labels: the names added and removed.
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
}

/// An issue's history as the detail reads it: the events, oldest first, and
/// whether the provider holds older entries than it returned (Linear's one
/// page is the newest entries), so the feed can say earlier changes are
/// missing instead of cutting them silently.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerHistory {
    pub events: Vec<TrackerHistoryEvent>,
    /// True when older entries exist that this read left out.
    #[serde(default)]
    pub more: bool,
}

/// One issue with every detail field the Tasks page shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerIssue {
    pub id: String,
    /// "ENG-37".
    pub key: String,
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub state: TrackerState,
    #[serde(default)]
    pub assignee: Option<TrackerUser>,
    /// 0 none, 1 urgent, 2 high, 3 medium, 4 low.
    #[serde(default)]
    pub priority: u8,
    /// A date (`YYYY-MM-DD`).
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub team_id: String,
    pub updated_at: String,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub trashed: bool,
    #[serde(default)]
    pub estimate: Option<f64>,
    #[serde(default)]
    pub cycle: Option<TrackerCycle>,
    #[serde(default)]
    pub milestone: Option<TrackerMilestone>,
    #[serde(default)]
    pub creator: Option<TrackerUser>,
    pub created_at: String,
    /// When the issue last reached a completed state (RFC 3339; G4's "Done
    /// after the run started"). None when it isn't Done, or for an issue
    /// mirrored before Hollow read this field (it fills at its next sync).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub completed_at: Option<String>,
    /// Sub-issues' keys.
    #[serde(default)]
    pub children: Vec<String>,
    #[serde(default)]
    pub relations: Vec<TrackerRelation>,
    #[serde(default)]
    pub links: Vec<TrackerLink>,
    #[serde(default)]
    pub attachments: Vec<TrackerLink>,
    #[serde(default)]
    pub comments: Vec<TrackerComment>,
}

/// A project's status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerProjectStatus {
    pub name: String,
    /// Linear's project status types: backlog, planned, started, paused,
    /// completed, canceled.
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerProject {
    pub id: String,
    /// "P-ENG-6", when the provider has one.
    #[serde(default)]
    pub key: Option<String>,
    pub url: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub status: TrackerProjectStatus,
    #[serde(default)]
    pub lead: Option<TrackerUser>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub target_date: Option<String>,
    #[serde(default)]
    pub team_ids: Vec<String>,
    pub updated_at: String,
    /// 0 to 1, as the provider reports it.
    #[serde(default)]
    pub progress: Option<f64>,
    #[serde(default)]
    pub links: Vec<TrackerLink>,
}

/// An issue field Hollow may write. The vault key is [`Field::key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "TrackerField"))]
#[serde(rename_all = "camelCase")]
pub enum Field {
    Title,
    Description,
    Status,
    Priority,
    Assignee,
    Labels,
    Project,
    Due,
    Parent,
    Estimate,
    Cycle,
    Milestone,
}

impl Field {
    pub const ALL: [Field; 12] = [
        Field::Title,
        Field::Description,
        Field::Status,
        Field::Priority,
        Field::Assignee,
        Field::Labels,
        Field::Project,
        Field::Due,
        Field::Parent,
        Field::Estimate,
        Field::Cycle,
        Field::Milestone,
    ];

    /// The task file's frontmatter key (design section 5).
    pub fn key(self) -> &'static str {
        match self {
            Field::Title => "title",
            Field::Description => "description",
            Field::Status => "status",
            Field::Priority => "priority",
            Field::Assignee => "assignee",
            Field::Labels => "labels",
            Field::Project => "project",
            Field::Due => "due",
            Field::Parent => "parent_key",
            Field::Estimate => "estimate",
            Field::Cycle => "cycle",
            Field::Milestone => "milestone",
        }
    }

    pub fn from_key(key: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key() == key)
    }
}

/// Task-file keys the tracker owns and Hollow never pushes: a local edit to
/// one is restored on the next pass (design section 5).
pub const READ_ONLY_KEYS: &[&str] = &[
    "creator",
    "created_at",
    "updated_at",
    "sub_issues",
    "blocked_by",
    "blocking",
    "related",
    "links",
    "attachments",
    "comment_count",
];

/// Every tracker-identity key on a mirrored task file.
pub const IDENTITY_KEYS: &[&str] = &[
    "tracker",
    "tracker_id",
    "tracker_key",
    "tracker_url",
    "tracker_team",
    "tracker_project_id",
    "tracker_state",
];

/// A new issue in the scoped team.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewIssue {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub state_id: Option<String>,
    #[serde(default)]
    pub priority: Option<u8>,
    #[serde(default)]
    pub assignee_id: Option<String>,
    #[serde(default)]
    pub label_ids: Vec<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub estimate: Option<f64>,
    #[serde(default)]
    pub cycle_id: Option<String>,
    #[serde(default)]
    pub milestone_id: Option<String>,
    /// The team to create it in (an id); None = the shared team (not
    /// serialized, so a shared-team request keeps its old journal key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// The issue's id, chosen by Hollow (a UUID v4) and kept in the create
    /// journal before the request (Q-R22): the provider refuses a second
    /// create with it, and recovery reads that one issue. Never read from or
    /// written to JSON, so no caller supplies it and journal keys keep.
    #[serde(skip)]
    pub id: Option<String>,
}

/// Where a create sent with Hollow's own issue id landed (Q-R22).
#[derive(Debug, Clone)]
pub enum CreateLanding {
    /// In the scoped team, read in full.
    InScope(TrackerIssue),
    /// It exists in another team: never adopted, never sent again.
    OtherTeam,
    /// No issue has that id anywhere: the create never reached the tracker.
    Absent,
}

/// A change to an issue. `None` leaves a field alone; `Some(None)` clears a
/// clearable one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IssuePatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub state_id: Option<String>,
    pub priority: Option<u8>,
    pub assignee_id: Option<Option<String>>,
    pub label_ids: Option<Vec<String>>,
    pub project_id: Option<Option<String>>,
    pub due: Option<Option<String>>,
    pub parent_id: Option<Option<String>>,
    pub estimate: Option<Option<f64>>,
    pub cycle_id: Option<Option<String>>,
    pub milestone_id: Option<Option<String>>,
}

impl IssuePatch {
    /// The fields this patch touches, in [`Field::ALL`] order.
    pub fn fields(&self) -> Vec<Field> {
        let mut out = Vec::new();
        let set = [
            (Field::Title, self.title.is_some()),
            (Field::Description, self.description.is_some()),
            (Field::Status, self.state_id.is_some()),
            (Field::Priority, self.priority.is_some()),
            (Field::Assignee, self.assignee_id.is_some()),
            (Field::Labels, self.label_ids.is_some()),
            (Field::Project, self.project_id.is_some()),
            (Field::Due, self.due.is_some()),
            (Field::Parent, self.parent_id.is_some()),
            (Field::Estimate, self.estimate.is_some()),
            (Field::Cycle, self.cycle_id.is_some()),
            (Field::Milestone, self.milestone_id.is_some()),
        ];
        for (field, touched) in set {
            if touched {
                out.push(field);
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.fields().is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // Project creation is part of the adapter contract; its user action follows A5.
pub struct NewProject {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub lead_id: Option<String>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub target_date: Option<String>,
    /// A project status id; the provider's default when absent.
    #[serde(default)]
    pub status_id: Option<String>,
    /// The team to create it in (an id); None = the shared team (not
    /// serialized, so a shared-team request keeps its old journal key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
#[allow(dead_code)] // Project write-through is retained for the A6 seams.
pub struct ProjectPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub lead_id: Option<Option<String>>,
    pub start_date: Option<Option<String>>,
    pub target_date: Option<Option<String>>,
    pub status_id: Option<String>,
}

/// A project's health on a status update (Linear's `onTrack`, `atRisk`,
/// `offTrack`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "TrackerProjectHealth"))]
#[serde(rename_all = "camelCase")]
pub enum ProjectHealth {
    OnTrack,
    AtRisk,
    OffTrack,
}

impl ProjectHealth {
    /// The provider word (`onTrack`, `atRisk`, `offTrack`).
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectHealth::OnTrack => "onTrack",
            ProjectHealth::AtRisk => "atRisk",
            ProjectHealth::OffTrack => "offTrack",
        }
    }

    /// What a person reads.
    pub fn label(self) -> &'static str {
        match self {
            ProjectHealth::OnTrack => "On track",
            ProjectHealth::AtRisk => "At risk",
            ProjectHealth::OffTrack => "Off track",
        }
    }
}

/// A status update posted on a project (A18's optional tracker copy).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerProjectUpdate {
    pub id: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// A file in the tracker's own storage, on no issue yet (A19): what
/// [`TrackerAdapter::upload_file`] returns and
/// [`TrackerAdapter::attach_upload`] puts on an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerUpload {
    /// Where the provider serves the file.
    pub asset_url: String,
    pub filename: String,
    pub size: u64,
}

/// What a link is added to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Canvas and migration links use both target kinds in later rows.
pub enum LinkTarget {
    Issue(String),
    Project(String),
}

/// One page of changes and where the next one starts.
#[derive(Debug, Clone, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// The cursor for the next page; None on the last.
    pub next: Option<String>,
}

/// The provider's rate-limit reading after the last request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "TrackerRateBudget"))]
#[serde(rename_all = "camelCase")]
pub struct RateBudget {
    #[serde(default)]
    pub requests_remaining: Option<u64>,
    #[serde(default)]
    pub requests_limit: Option<u64>,
    #[serde(default)]
    pub complexity_remaining: Option<u64>,
    #[serde(default)]
    pub complexity_limit: Option<u64>,
    /// When the window resets (RFC 3339).
    #[serde(default)]
    pub reset_at: Option<String>,
}

impl RateBudget {
    /// Under 10% of either budget left: the next pass waits for the reset.
    pub fn is_low(&self) -> bool {
        let low = |left: Option<u64>, limit: Option<u64>| match (left, limit) {
            (Some(left), Some(limit)) if limit > 0 => left * 10 < limit,
            _ => false,
        };
        low(self.requests_remaining, self.requests_limit)
            || low(self.complexity_remaining, self.complexity_limit)
    }
}

/// Why a tracker call failed. Messages never carry the key or issue text.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackerError {
    /// No provider chosen, no team set, or no key saved.
    NotConfigured(String),
    /// The key was refused.
    Auth,
    /// The node or mutation is outside the scoped team; nothing was written.
    OutOfScope,
    RateLimited {
        reset_at: Option<String>,
    },
    /// Both sides changed; the tracker's copy wins.
    #[allow(dead_code)] // Alternative adapters may return an optimistic-write conflict.
    Conflict {
        remote: Box<TrackerIssue>,
    },
    NotFound,
    Network(String),
    Provider(String),
}

impl std::fmt::Display for TrackerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrackerError::NotConfigured(why) => write!(f, "The task tracker isn't set up: {why}"),
            TrackerError::Auth => f.write_str(
                "The task tracker refused the saved API key. Save a new one in Settings, Sources, Task tracker.",
            ),
            TrackerError::OutOfScope => f.write_str(
                "That item belongs to another team; Hollow only reads and writes its own team.",
            ),
            TrackerError::RateLimited { reset_at: Some(at) } => {
                write!(f, "The task tracker asked Hollow to slow down until {at}.")
            }
            TrackerError::RateLimited { reset_at: None } => {
                f.write_str("The task tracker asked Hollow to slow down; it will try again shortly.")
            }
            TrackerError::Conflict { .. } => f.write_str(
                "The task tracker changed this item at the same time; its version was kept.",
            ),
            TrackerError::NotFound => f.write_str("The task tracker has no such item."),
            TrackerError::Network(why) => write!(f, "Couldn't reach the task tracker: {why}"),
            TrackerError::Provider(why) => write!(f, "The task tracker reported an error: {why}"),
        }
    }
}

impl std::error::Error for TrackerError {}

/// What every adapter call returns.
pub type TrackerFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, TrackerError>> + Send + 'a>>;

/// One provider. Every call is scoped to the configured team: a node from
/// another team is dropped (and logged as `tracker: dropped out-of-scope
/// node`), and a mutation reads its target's team first and fails with
/// [`TrackerError::OutOfScope`] unless it matches.
#[allow(dead_code)] // Keep the complete adapter contract while A6 and attachment writes are staged.
pub trait TrackerAdapter: Send + Sync {
    /// The provider id in config (`tracker.provider`).
    fn provider(&self) -> &'static str;
    /// The name people see ("Open in …").
    fn display_name(&self) -> &str;
    /// The same provider scoped to another team it may touch (the second
    /// team, 2026-11-12). None: this adapter already is that team, or the
    /// provider has one team only; an Err refuses a team the provider's guard
    /// doesn't allow.
    fn for_team(&self, _team: &str) -> Option<Result<Box<dyn TrackerAdapter>, TrackerError>> {
        None
    }
    /// The issue fields this provider accepts from Hollow.
    fn writable_fields(&self) -> &[Field];
    /// The rate-limit reading after the last request, when the provider sends one.
    fn rate(&self) -> Option<RateBudget>;

    /// The key's own user.
    fn whoami(&self) -> TrackerFuture<'_, TrackerUser>;
    /// The scoped team, read back from the provider (proves the key sees it).
    fn scope(&self) -> TrackerFuture<'_, TrackerTeam>;
    fn states(&self) -> TrackerFuture<'_, Vec<TrackerState>>;
    fn labels(&self) -> TrackerFuture<'_, Vec<TrackerLabel>>;
    fn members(&self) -> TrackerFuture<'_, Vec<TrackerUser>>;
    fn cycles(&self) -> TrackerFuture<'_, Vec<TrackerCycle>>;
    /// Milestones of the given projects.
    fn milestones<'a>(
        &'a self,
        project_ids: &'a [String],
    ) -> TrackerFuture<'a, Vec<TrackerMilestone>>;

    /// Issues updated after `since` (RFC 3339; None = all), archived ones
    /// included, one page from `after`, oldest change first within the page.
    /// List issues carry no comments or attachments (those come from
    /// [`TrackerAdapter::comments_changed`] and [`TrackerAdapter::attachments`]);
    /// the engine moves its cursor only after the last page.
    fn issues_changed<'a>(
        &'a self,
        since: Option<&'a str>,
        after: Option<&'a str>,
    ) -> TrackerFuture<'a, Page<TrackerIssue>>;
    /// Comments updated after `since`, on the scoped team's issues.
    fn comments_changed<'a>(
        &'a self,
        since: Option<&'a str>,
        after: Option<&'a str>,
    ) -> TrackerFuture<'a, Page<TrackerCommentChange>>;
    /// Links and attachments of the given issues, by issue id.
    fn attachments<'a>(
        &'a self,
        issue_ids: &'a [String],
    ) -> TrackerFuture<'a, Vec<(String, Vec<TrackerLink>)>>;
    /// Projects the scoped team can see, updated after `since`.
    fn projects_changed<'a>(
        &'a self,
        since: Option<&'a str>,
    ) -> TrackerFuture<'a, Vec<TrackerProject>>;
    /// One issue with every detail field.
    fn issue<'a>(&'a self, id: &'a str) -> TrackerFuture<'a, TrackerIssue>;

    /// Complete scoped comments. Partial or malformed collections must refuse.
    fn issue_comments<'a>(&'a self, _id: &'a str) -> TrackerFuture<'a, Vec<TrackerCommentChange>> {
        Box::pin(async {
            Err(TrackerError::Provider(
                "Complete comment reads are unsupported.".into(),
            ))
        })
    }
    /// An issue of the scoped team's change history (state, assignee,
    /// priority, labels, project, title, description), oldest first, read on
    /// demand for the issue detail's activity, with `more` set when older
    /// entries were left out. Never stored in the mirror.
    fn issue_history<'a>(&'a self, _id: &'a str) -> TrackerFuture<'a, TrackerHistory> {
        Box::pin(async {
            Err(TrackerError::Provider(
                "Issue history reads are unsupported.".into(),
            ))
        })
    }

    /// Internal host seam, with a fresh authority check after the scope read.
    /// Implementations MUST NOT send a mutation before `authorize` returns Ok.
    /// Failure before that callback succeeds is a proven no-send outcome;
    /// failure afterward is conservatively uncertain regardless of HTTP status.
    /// No IPC or agent action exposes this method.
    fn create_issue_comment<'a>(
        &'a self,
        _id: &'a str,
        _body: &'a str,
        _authorize: &'a (dyn Fn() -> Result<(), TrackerError> + Send + Sync),
    ) -> TrackerFuture<'a, TrackerCommentChange> {
        Box::pin(async {
            Err(TrackerError::Provider(
                "Comment publication is unsupported.".into(),
            ))
        })
    }

    /// Post a status update on a project of the scoped team alone (A18's
    /// optional Friday status copy). Same contract as
    /// [`TrackerAdapter::create_issue_comment`]: the target's team is read
    /// first, and nothing is sent before `authorize` returns Ok.
    fn post_project_update<'a>(
        &'a self,
        _project_id: &'a str,
        _body: &'a str,
        _health: ProjectHealth,
        _authorize: &'a (dyn Fn() -> Result<(), TrackerError> + Send + Sync),
    ) -> TrackerFuture<'a, TrackerProjectUpdate> {
        Box::pin(async {
            Err(TrackerError::Provider(
                "Project status updates are unsupported.".into(),
            ))
        })
    }

    /// Create one label in the scoped team (T1's `tracker.ensure_labels`),
    /// never a workspace label. The caller reads [`TrackerAdapter::labels`]
    /// first and creates only the missing ones.
    fn create_label<'a>(&'a self, _name: &'a str) -> TrackerFuture<'a, TrackerLabel> {
        Box::pin(async {
            Err(TrackerError::Provider(
                "Label creation is unsupported.".into(),
            ))
        })
    }

    /// Put a file in the provider's storage (A19). It is on no issue yet:
    /// [`TrackerAdapter::attach_upload`] puts it on one.
    fn upload_file<'a>(
        &'a self,
        _filename: &'a str,
        _content_type: &'a str,
        _bytes: &'a [u8],
    ) -> TrackerFuture<'a, TrackerUpload> {
        Box::pin(async {
            Err(TrackerError::Provider(
                "File uploads are unsupported.".into(),
            ))
        })
    }

    /// Attach an uploaded file to an issue of the scoped team (A19). The
    /// target's team is read first and nothing is sent before `authorize`
    /// returns Ok. Must be idempotent per issue and upload: a second call
    /// with the same upload updates the attachment, never adds another.
    fn attach_upload<'a>(
        &'a self,
        _issue_id: &'a str,
        _upload: &'a TrackerUpload,
        _title: &'a str,
        _authorize: &'a (dyn Fn() -> Result<(), TrackerError> + Send + Sync),
    ) -> TrackerFuture<'a, TrackerLink> {
        Box::pin(async {
            Err(TrackerError::Provider(
                "File attachments are unsupported.".into(),
            ))
        })
    }

    fn create_issue(&self, issue: NewIssue) -> TrackerFuture<'_, TrackerIssue>;
    /// Where a create sent with Hollow's own issue id landed (Q-R22): read
    /// by that id in any team, id and team only, so a moved issue is told
    /// apart from one that never arrived. Only a complete, well-formed empty
    /// answer is [`CreateLanding::Absent`]; anything unclear is an error.
    fn create_landing<'a>(&'a self, _id: &'a str) -> TrackerFuture<'a, CreateLanding> {
        Box::pin(async {
            Err(TrackerError::Provider(
                "Create read-back is unsupported.".into(),
            ))
        })
    }
    fn update_issue<'a>(
        &'a self,
        id: &'a str,
        patch: IssuePatch,
    ) -> TrackerFuture<'a, TrackerIssue>;
    fn archive_issue<'a>(&'a self, id: &'a str) -> TrackerFuture<'a, ()>;
    fn create_project(&self, project: NewProject) -> TrackerFuture<'_, TrackerProject>;
    fn update_project<'a>(
        &'a self,
        id: &'a str,
        patch: ProjectPatch,
    ) -> TrackerFuture<'a, TrackerProject>;
    /// Append a link (the Notion archive link on an issue; GitHub and Live
    /// URL on a project). Never removes one.
    fn add_link<'a>(
        &'a self,
        target: LinkTarget,
        url: &'a str,
        title: &'a str,
    ) -> TrackerFuture<'a, TrackerLink>;
}

/// The scope guard's filter, shared by every adapter: keep nodes of `team`,
/// drop the rest with one log line each (ids only, never text).
pub fn keep_in_scope<T>(
    team_id: &str,
    nodes: Vec<T>,
    team_of: impl Fn(&T) -> &str,
    id_of: impl Fn(&T) -> &str,
) -> Vec<T> {
    nodes
        .into_iter()
        .filter(|node| {
            let keep = team_of(node) == team_id;
            if !keep {
                eprintln!("[tracker] dropped out-of-scope node {}", id_of(node));
            }
            keep
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_kinds_parse_provider_words_and_drive_done_logic() {
        assert_eq!(StateKind::parse("Completed"), Some(StateKind::Completed));
        assert_eq!(StateKind::parse(" triage "), Some(StateKind::Backlog));
        assert_eq!(StateKind::parse("cancelled"), Some(StateKind::Canceled));
        assert_eq!(StateKind::parse("Delivered"), None, "names never count");
        for kind in StateKind::ALL {
            assert_eq!(StateKind::parse(kind.as_str()), Some(kind));
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::json!(kind.as_str())
            );
        }
        assert!(StateKind::Completed.is_done());
        assert!(!StateKind::Canceled.is_done());
        assert!(StateKind::Canceled.is_closed());
        assert!(!StateKind::Started.is_closed());
    }

    #[test]
    fn fields_round_trip_their_vault_keys_and_never_overlap_read_only_keys() {
        for field in Field::ALL {
            assert_eq!(Field::from_key(field.key()), Some(field));
            assert!(!READ_ONLY_KEYS.contains(&field.key()), "{field:?}");
            assert!(!IDENTITY_KEYS.contains(&field.key()), "{field:?}");
        }
        assert_eq!(Field::from_key("creator"), None);
    }

    #[test]
    fn a_patch_reports_exactly_the_fields_it_touches() {
        let patch = IssuePatch {
            title: Some("New".into()),
            due: Some(None),
            milestone_id: Some(Some("m1".into())),
            ..IssuePatch::default()
        };
        assert_eq!(
            patch.fields(),
            vec![Field::Title, Field::Due, Field::Milestone]
        );
        assert!(IssuePatch::default().is_empty());
    }

    #[test]
    fn a_low_budget_is_under_ten_percent_of_either_limit() {
        let budget = |rl, rr, cl, cr| RateBudget {
            requests_limit: rl,
            requests_remaining: rr,
            complexity_limit: cl,
            complexity_remaining: cr,
            reset_at: None,
        };
        assert!(!budget(Some(2500), Some(2400), Some(3_000_000), Some(2_000_000)).is_low());
        assert!(budget(Some(2500), Some(249), None, None).is_low());
        assert!(budget(None, None, Some(3_000_000), Some(299_999)).is_low());
        assert!(
            !budget(None, Some(1), None, None).is_low(),
            "no limit known"
        );
        assert!(!RateBudget::default().is_low());
    }

    #[test]
    fn the_scope_filter_drops_other_teams() {
        let nodes = vec![("a", "ENG"), ("b", "LP"), ("c", "ENG")];
        let kept = keep_in_scope("ENG", nodes, |n| n.1, |n| n.0);
        assert_eq!(kept, vec![("a", "ENG"), ("c", "ENG")]);
    }

    #[test]
    fn errors_never_echo_more_than_their_own_words() {
        let shown = TrackerError::Auth.to_string();
        assert!(shown.contains("Settings, Sources, Task tracker"));
        assert!(TrackerError::RateLimited {
            reset_at: Some("2026-11-08T05:00:00Z".into())
        }
        .to_string()
        .contains("2026-11-08T05:00:00Z"));
        for error in [
            TrackerError::NotConfigured("no key".into()),
            TrackerError::OutOfScope,
            TrackerError::NotFound,
            TrackerError::Network("timeout".into()),
            TrackerError::Provider("bad".into()),
        ] {
            assert!(
                !error.to_string().to_ascii_lowercase().contains("linear"),
                "{error}"
            );
        }
    }
}
