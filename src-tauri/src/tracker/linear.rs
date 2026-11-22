//! The Linear adapter (overhaul A3; contract `docs/TRACKER_SYNC.md`).
//!
//! The one file that names Linear: its constants (provider id, display name,
//! endpoint, the Keychain item) and [`LinearAdapter`], the [`TrackerAdapter`]
//! over Linear's GraphQL API with the team-restricted personal API key.
//!
//! - **Scope guard.** Every issue, comment, project and label query names the
//!   configured team in its filter, so the server answers for that team
//!   only; a node that still carries another team is dropped (one log line,
//!   ids only). A mutation first reads its target through the same team
//!   filter and refuses with [`TrackerError::OutOfScope`] when nothing comes
//!   back, so Hollow never reads another team's issue even to check it, and
//!   the mutation's reply is checked again. A project shared with another
//!   team is read but never written.
//! - **Rate limits.** Every reply's `X-RateLimit-*` headers land in
//!   [`TrackerAdapter::rate`] (the engine waits for the reset under 10%
//!   left). A 429 or `RATELIMITED` reply backs off: 30 s, doubling to 15
//!   minutes; a call inside the window fails at once without a request, and
//!   a good reply clears it.
//! - **Points.** Linear refuses a query over 10,000 complexity points. The
//!   page sizes below keep every query under it, which
//!   `every_query_stays_under_the_point_cap` checks with Linear's published
//!   rules, counted generously.
//! - **The key** goes in the `Authorization` header as is (marked sensitive)
//!   and nowhere else: never in a log, an error or `Debug`.

use super::settings::{self, TrackerConfig};
use super::{
    keep_in_scope, CreateLanding, Field, HistoryKind, IssuePatch, LinkTarget, NewIssue, NewProject,
    Page, ProjectHealth, ProjectPatch, RateBudget, RelationKind, StateKind, TrackerAdapter,
    TrackerComment, TrackerCommentChange, TrackerCycle, TrackerError, TrackerFuture,
    TrackerHistory, TrackerHistoryEvent, TrackerIssue, TrackerLabel, TrackerLink, TrackerMilestone,
    TrackerProject, TrackerProjectStatus, TrackerProjectUpdate, TrackerRelation, TrackerState,
    TrackerTeam, TrackerUpload, TrackerUser,
};
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// `tracker.provider` in config.
pub const PROVIDER: &str = "linear";
/// Static capabilities also support older ledgers before their first online pass.
pub(crate) const WRITABLE_FIELDS: &[Field] = &Field::ALL;

/// The name people see ("Open in Linear").
pub const DISPLAY_NAME: &str = "Linear";

/// The GraphQL endpoint. The key goes in the `Authorization` header as is.
pub const API_URL: &str = "https://api.linear.app/graphql";

/// The Keychain item holding the team-restricted API key (service, account),
/// created by hand on 2026-11-07. Settings' "Use the key saved in Keychain"
/// copies it into Hollow's own bundle once.
pub const KEYCHAIN_ITEM: (&str, &str) = ("hollow-linear", "default");

/// Transports this adapter offers: the GraphQL API with an API key, or
/// Linear's MCP server over Hollow's `linear` sign-in (the fallback, A4).
pub const TRANSPORTS: &[&str] = &["graphql", "mcp"];

/// The shared team (Engineering, key `ENG`): real project work, seen by
/// the department. Hollow's primary team and the only one a config may
/// name as `teamId`.
pub const TEAM_ID: &str = "11111111-1111-4111-8111-111111111111";
/// The shared team's key.
pub const TEAM_KEY: &str = "ENG";
/// The personal team (Personal, key `PER`, 2026-11-12): items that only
/// involve Alex and Hollow. Opt-in through the config's
/// `personalTeamId`; the second and last team an adapter accepts.
pub const PERSONAL_TEAM_ID: &str = "22222222-2222-4222-8222-222222222222";
/// The personal team's key.
pub const PERSONAL_TEAM_KEY: &str = "PER";
/// Why another team is refused, in plain words.
pub const TEAM_ONLY: &str = "Hollow works only with team Engineering and team Personal.";

/// The key of an allowed team by its id (`ENG`, `PER`).
pub fn team_key_of(id: &str) -> Option<&'static str> {
    match id {
        TEAM_ID => Some(TEAM_KEY),
        PERSONAL_TEAM_ID => Some(PERSONAL_TEAM_KEY),
        _ => None,
    }
}

/// Whether `id` is one of the two teams Hollow may touch (by id).
pub fn team_allowed(id: &str) -> bool {
    id == TEAM_ID || id == PERSONAL_TEAM_ID
}

/// The team id for a team key (`ENG` or `PER`, any case) or an id of one of
/// the two allowed teams; None for anything else.
pub fn allowed_team(text: &str) -> Option<&'static str> {
    if text == TEAM_ID || text.eq_ignore_ascii_case(TEAM_KEY) {
        Some(TEAM_ID)
    } else if text == PERSONAL_TEAM_ID || text.eq_ignore_ascii_case(PERSONAL_TEAM_KEY) {
        Some(PERSONAL_TEAM_ID)
    } else {
        None
    }
}

/// What a new tracker item is about, for [`team_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    /// Real project work the department sees (team AI).
    Tracked,
    /// Work that involves only Alex and Hollow: Hollow's own build
    /// items, agent bookkeeping, his private commitments (team PER).
    HollowOnly,
}

impl Work {
    /// Hollow-only when any label is one only team PER keeps
    /// ([`HOLLOW_ONLY_LABELS`], any case); tracked otherwise.
    pub fn of_labels(labels: &[String]) -> Work {
        let hollow_only = labels.iter().any(|label| {
            HOLLOW_ONLY_LABELS
                .iter()
                .any(|own| own.eq_ignore_ascii_case(label.trim()))
        });
        if hollow_only {
            Work::HollowOnly
        } else {
            Work::Tracked
        }
    }
}

/// Team routing (two-Macs T1, 2026-11-15). The rule: tracked work goes to
/// team ENG and Hollow-only work to team PER; a team the request names wins
/// when it is AI or HOL (by key, any case, or id), and any other team (LP,
/// PRO, the rest) is refused. Returns the team id.
pub fn team_for(work: Work, named: Option<&str>) -> Result<&'static str, String> {
    match named.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => allowed_team(name).ok_or_else(|| format!("{TEAM_ONLY} Nothing was sent.")),
        None => Ok(match work {
            Work::Tracked => TEAM_ID,
            Work::HollowOnly => PERSONAL_TEAM_ID,
        }),
    }
}

/// Labels that route an unnamed create to team PER (D8 and two-Macs T2,
/// 2026-11-12): a commitment, agent bookkeeping and a Hollow lesson involve
/// only Alex and Hollow by default. A commitment that involves the team
/// lands in AI too (T3e), so AI also keeps `commitment`.
pub const HOLLOW_ONLY_LABELS: &[&str] = &["commitment", "agent", "hollow-lesson"];
/// The labels Hollow keeps in team ENG: `needs-review` on shared issues
/// waiting on him (D8) and `commitment` on a team commitment of his (T3e).
pub const AI_LABELS: &[&str] = &["needs-review", "commitment"];
/// The labels Hollow keeps in team PER.
pub const HOL_LABELS: &[&str] = &["needs-review", "commitment", "agent", "hollow-lesson"];

/// The labels `tracker.ensure_labels` keeps in a team, by its id (none for
/// a team that isn't AI or HOL).
pub fn labels_for(team_id: &str) -> &'static [&'static str] {
    match team_id {
        TEAM_ID => AI_LABELS,
        PERSONAL_TEAM_ID => HOL_LABELS,
        _ => &[],
    }
}

/// Linear refuses a query over this many complexity points.
#[cfg(test)]
pub const POINT_CAP: u64 = 10_000;

/// Issues per page of the incremental pass. With their nested lists at
/// [`NESTED_PAGE`] a page stays under [`POINT_CAP`]; Linear's usual 100 a
/// page would not.
const ISSUE_PAGE: u32 = 40;
/// Labels, sub-issues and relations read per issue in a list. A list that
/// comes back full may have more, so that issue is read again in detail.
const NESTED_PAGE: u32 = 20;
/// The detail query's lists: labels, sub-issues, relations, comments and
/// attachments.
const DETAIL_PAGE: u32 = 50;
const COMMENT_PAGE: u32 = 100;
/// History entries read for an issue detail's activity (the latest Linear
/// returns first; the detail shows them with its comments). No next page is
/// read: `hasNextPage` becomes the history's `more`, and the feed says so.
const HISTORY_PAGE: u32 = 50;
const META_PAGE: u32 = 100;
const PROJECT_PAGE: u32 = 50;
/// Issues per attachments query.
const BATCH: usize = 50;
/// Projects per milestones query. Linear counted 5,555 points for 50 (live,
/// 2026-11-08), more than this file's counter says for that doubly nested
/// shape, so 20 keeps it near 2,200.
const MILESTONE_BATCH: usize = 20;
/// Pages one call reads before it gives up (a runaway cursor).
const MAX_PAGES: usize = 50;
/// How far back the team's cycles are read (current and upcoming ones, plus
/// the ones that just ended).
const CYCLE_DAYS_BACK: i64 = 30;
const BACKOFF_BASE: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(15 * 60);
const TIMEOUT: Duration = Duration::from_secs(30);

// ---- Query text --------------------------------------------------------------

const PAGE_INFO: &str = "pageInfo { hasNextPage endCursor }";
const ATTACHMENT_FIELDS: &str = "id title url sourceType createdAt";
const PROJECT_FIELDS: &str =
    "id identifier url name description startDate targetDate updatedAt progress \
     status { name type } lead { id name email } teams(first: 10) { nodes { id } } \
     externalLinks(first: 20) { nodes { id label url createdAt } }";

const VIEWER_QUERY: &str = "query Viewer { viewer { id name email } }";
const TEAM_QUERY: &str = "query Team($team: String!) { team(id: $team) { id key name } }";
const ISSUE_TARGET_QUERY: &str = "query IssueTarget($filter: IssueFilter) { issues(filter: $filter, first: 1, includeArchived: true) { nodes { id identifier team { id } } } }";
/// Q-R22's read-back of a create by Hollow's own id. It names no team on
/// purpose, so an issue moved to another team is told apart from one that
/// never arrived; it selects only the id and the team id.
const TEAMS_VISIBLE_QUERY: &str = "query TeamsVisible { teams(first: 250) { nodes { id } } }";
const ISSUE_LANDING_QUERY: &str = "query IssueLanding($id: ID!) { issues(filter: {id: {eq: $id}}, first: 2, includeArchived: true) { nodes { id team { id } } } }";
const PROJECT_TARGET_QUERY: &str = "query ProjectTarget($filter: ProjectFilter) { projects(filter: $filter, first: 1) { nodes { id teams(first: 10) { nodes { id } } } } }";
const ISSUE_ARCHIVE_MUTATION: &str =
    "mutation IssueArchive($id: String!) { issueArchive(id: $id) { success } }";
const PROJECT_LINK_MUTATION: &str = "mutation ProjectLink($input: EntityExternalLinkCreateInput!) { entityExternalLinkCreate(input: $input) { success entityExternalLink { id label url createdAt } } }";
const COMMENT_CREATE_MUTATION: &str = "mutation CommentCreate($input: CommentCreateInput!) { commentCreate(input: $input) { success comment { id body createdAt updatedAt editedAt parent { id } user { id name } issue { id team { id } } } } }";
/// A18: a project status update (Linear's `projectUpdateCreate`; not
/// `projectUpdate`, which edits the project's own fields).
const STATUS_UPDATE_MUTATION: &str = "mutation ProjectStatusUpdate($input: ProjectUpdateCreateInput!) { projectUpdateCreate(input: $input) { success projectUpdate { id url createdAt project { id } } } }";
/// T1: one team label (`issueLabelCreate` with the scoped team's id, so
/// never a workspace label).
const LABEL_CREATE_MUTATION: &str = "mutation LabelCreate($input: IssueLabelCreateInput!) { issueLabelCreate(input: $input) { success issueLabel { id name color team { id } parent { name } } } }";
/// A19: a signed upload address for one file (`fileUpload`).
const FILE_UPLOAD_MUTATION: &str = "mutation FileUpload($contentType: String!, $filename: String!, $size: Int!) { fileUpload(contentType: $contentType, filename: $filename, size: $size) { success uploadFile { uploadUrl assetUrl headers { key value } } } }";
/// A19: the uploaded file on an issue. Linear updates the attachment with
/// the same URL on the same issue rather than adding a second.
const ATTACHMENT_CREATE_MUTATION: &str = "mutation AttachmentCreate($input: AttachmentCreateInput!) { attachmentCreate(input: $input) { success attachment { id title url sourceType createdAt issue { id team { id } } } } }";
/// The largest file Hollow uploads (a canvas is a few hundred KB at most).
pub(crate) const UPLOAD_MAX_BYTES: usize = 10 * 1024 * 1024;

/// An issue's fields, with its nested lists read `nested` at a time.
fn issue_fields(nested: u32) -> String {
    format!(
        "id identifier url title description priority dueDate estimate createdAt updatedAt \
         completedAt archivedAt trashed team {{ id }} state {{ id name type position color }} \
         assignee {{ id name email }} creator {{ id name }} project {{ id }} \
         parent {{ id team {{ id }} }} cycle {{ id number name startsAt endsAt }} \
         projectMilestone {{ id name targetDate project {{ id }} }} \
         labels(first: {nested}) {{ nodes {{ name }} }} \
         children(first: {nested}) {{ nodes {{ identifier team {{ id }} }} }} \
         relations(first: {nested}) {{ nodes {{ type relatedIssue {{ identifier team {{ id }} }} }} }} \
         inverseRelations(first: {nested}) {{ nodes {{ type issue {{ identifier team {{ id }} }} }} }}"
    )
}

fn issues_query() -> String {
    format!(
        "query Issues($filter: IssueFilter, $after: String) {{ issues(filter: $filter, \
         first: {ISSUE_PAGE}, after: $after, orderBy: updatedAt, includeArchived: true) \
         {{ nodes {{ {} }} {PAGE_INFO} }} }}",
        issue_fields(NESTED_PAGE)
    )
}

fn detail_query() -> String {
    format!(
        "query IssueDetail($filter: IssueFilter) {{ issues(filter: $filter, first: 1, \
         includeArchived: true) {{ nodes {{ {} \
         comments(first: {DETAIL_PAGE}) {{ nodes {{ id body createdAt editedAt parent {{ id }} user {{ id name }} }} }} \
         attachments(first: {DETAIL_PAGE}) {{ nodes {{ {ATTACHMENT_FIELDS} }} }} }} }} }}",
        issue_fields(DETAIL_PAGE)
    )
}

/// One issue's history, through the same team filter as the detail, so
/// another team's issue answers with nothing.
fn history_query() -> String {
    format!(
        "query IssueHistory($filter: IssueFilter) {{ issues(filter: $filter, first: 1, \
         includeArchived: true) {{ nodes {{ id identifier team {{ id }} \
         history(first: {HISTORY_PAGE}) {{ nodes {{ id createdAt actor {{ name }} \
         fromState {{ name type }} toState {{ name type }} fromAssignee {{ name }} \
         toAssignee {{ name }} fromPriority toPriority addedLabels {{ name }} \
         removedLabels {{ name }} fromProject {{ name }} toProject {{ name }} fromTitle \
         toTitle updatedDescription }} pageInfo {{ hasNextPage }} }} }} }} }}"
    )
}

fn comments_query() -> String {
    format!(
        "query Comments($filter: CommentFilter, $after: String) {{ comments(filter: $filter, \
         first: {COMMENT_PAGE}, after: $after, orderBy: updatedAt) {{ nodes {{ id body createdAt \
         updatedAt editedAt parent {{ id }} user {{ id name }} issue {{ id team {{ id }} }} }} \
         {PAGE_INFO} }} }}"
    )
}

fn attachments_query() -> String {
    format!(
        "query Attachments($filter: IssueFilter) {{ issues(filter: $filter, first: {BATCH}, \
         includeArchived: true) {{ nodes {{ id team {{ id }} attachments(first: {DETAIL_PAGE}) \
         {{ nodes {{ {ATTACHMENT_FIELDS} }} }} }} }} }}"
    )
}

fn projects_query() -> String {
    format!(
        "query Projects($filter: ProjectFilter, $after: String) {{ projects(filter: $filter, \
         first: {PROJECT_PAGE}, after: $after) {{ nodes {{ {PROJECT_FIELDS} }} {PAGE_INFO} }} }}"
    )
}

fn milestones_query() -> String {
    format!(
        "query Milestones($filter: ProjectFilter) {{ projects(filter: $filter, first: {MILESTONE_BATCH}) \
         {{ nodes {{ id projectMilestones(first: {DETAIL_PAGE}) {{ nodes {{ id name targetDate }} }} }} }} }}"
    )
}

fn states_query() -> String {
    format!(
        "query States($team: String!, $after: String) {{ team(id: $team) {{ id \
         states(first: {META_PAGE}, after: $after) {{ nodes {{ id name type position color \
         team {{ id }} }} {PAGE_INFO} }} }} }}"
    )
}

fn labels_query() -> String {
    format!(
        "query Labels($filter: IssueLabelFilter, $after: String) {{ issueLabels(filter: $filter, \
         first: {META_PAGE}, after: $after) {{ nodes {{ id name color isGroup team {{ id }} \
         parent {{ name }} }} {PAGE_INFO} }} }}"
    )
}

fn members_query() -> String {
    format!(
        "query Members($team: String!, $after: String) {{ team(id: $team) {{ id \
         members(first: {META_PAGE}, after: $after) {{ nodes {{ id name displayName email active }} \
         {PAGE_INFO} }} }} }}"
    )
}

fn cycles_query() -> String {
    format!(
        "query Cycles($team: String!, $filter: CycleFilter, $after: String) {{ team(id: $team) \
         {{ id cycles(filter: $filter, first: {META_PAGE}, after: $after) {{ nodes {{ id number \
         name startsAt endsAt }} {PAGE_INFO} }} }} }}"
    )
}

fn issue_create_mutation() -> String {
    format!(
        "mutation IssueCreate($input: IssueCreateInput!) {{ issueCreate(input: $input) \
         {{ success issue {{ {} }} }} }}",
        issue_fields(NESTED_PAGE)
    )
}

fn issue_update_mutation() -> String {
    format!(
        "mutation IssueUpdate($id: String!, $input: IssueUpdateInput!) {{ issueUpdate(id: $id, \
         input: $input) {{ success issue {{ {} }} }} }}",
        issue_fields(NESTED_PAGE)
    )
}

fn project_create_mutation() -> String {
    format!(
        "mutation ProjectCreate($input: ProjectCreateInput!) {{ projectCreate(input: $input) \
         {{ success project {{ {PROJECT_FIELDS} }} }} }}"
    )
}

fn project_update_mutation() -> String {
    format!(
        "mutation ProjectUpdate($id: String!, $input: ProjectUpdateInput!) {{ projectUpdate(id: \
         $id, input: $input) {{ success project {{ {PROJECT_FIELDS} }} }} }}"
    )
}

fn issue_link_mutation() -> String {
    format!(
        "mutation IssueLink($issueId: String!, $url: String!, $title: String) {{ \
         attachmentLinkURL(issueId: $issueId, url: $url, title: $title) {{ success attachment \
         {{ {ATTACHMENT_FIELDS} }} }} }}"
    )
}

// ---- The adapter ---------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct Backoff {
    /// Rate-limited replies in a row.
    streak: u32,
    /// No request before this.
    until: Option<DateTime<Utc>>,
}

/// The wait after the `streak`th rate-limited reply in a row: `base`,
/// doubling, never more than `max`.
fn backoff_delay(base: Duration, max: Duration, streak: u32) -> Duration {
    let doublings = streak.saturating_sub(1).min(31);
    base.saturating_mul(1u32 << doublings).min(max)
}

/// The Linear adapter over GraphQL, scoped to one team.
pub struct LinearAdapter {
    http: reqwest::Client,
    url: String,
    auth: HeaderValue,
    team_id: String,
    /// The config set up the personal team (`tracker.graphql` allows its
    /// team only then).
    personal: bool,
    rate: Mutex<Option<RateBudget>>,
    points: Mutex<Option<u64>>,
    backoff: Mutex<Backoff>,
    backoff_base: Duration,
    backoff_max: Duration,
    /// The key's organization has been confirmed as Acme's (R3, Q-R25): a
    /// write asks once per adapter, and a new key is a new adapter.
    org_pinned: Mutex<bool>,
}

impl std::fmt::Debug for LinearAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinearAdapter")
            .field("url", &self.url)
            .field("team_id", &self.team_id)
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl LinearAdapter {
    /// The adapter for `team_id` with `key`, against Linear's API.
    pub fn new(team_id: &str, key: &str) -> Result<Self, TrackerError> {
        Self::build(API_URL, team_id, key)
    }

    /// The same against the tests' stub server (the key only ever goes to
    /// Linear's own endpoint outside tests).
    #[cfg(test)]
    pub(crate) fn with_url(url: &str, team_id: &str, key: &str) -> Result<Self, TrackerError> {
        // The stub servers don't answer the org pin; `pin_org` brings it back
        // for the tests of the pin itself.
        Self::build(url, team_id, key).inspect(|a| {
            *lock(&a.org_pinned) = true;
        })
    }

    /// A test adapter that asks for the organization before its first write.
    #[cfg(test)]
    pub(crate) fn pin_org(self) -> Self {
        *lock(&self.org_pinned) = false;
        self
    }

    fn build(url: &str, team_id: &str, key: &str) -> Result<Self, TrackerError> {
        let team_id = team_id.trim();
        if team_id.is_empty() {
            return Err(TrackerError::NotConfigured("no team is set".into()));
        }
        if !settings::looks_like_id(team_id) {
            // A key ("ENG") in place of the id would match nothing anywhere.
            return Err(TrackerError::NotConfigured(
                "the team is set by its key, not its id".into(),
            ));
        }
        if !team_allowed(team_id) {
            return Err(TrackerError::NotConfigured(TEAM_ONLY.into()));
        }
        let key = key.trim();
        if key.is_empty() {
            return Err(TrackerError::NotConfigured("no API key is saved".into()));
        }
        let mut auth = HeaderValue::from_str(key).map_err(|_| {
            TrackerError::NotConfigured("the saved API key has characters a key can't have".into())
        })?;
        auth.set_sensitive(true);
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(network)?;
        Ok(LinearAdapter {
            http,
            url: url.to_string(),
            auth,
            team_id: team_id.to_string(),
            personal: false,
            rate: Mutex::new(None),
            points: Mutex::new(None),
            backoff: Mutex::new(Backoff::default()),
            backoff_base: BACKOFF_BASE,
            backoff_max: BACKOFF_MAX,
            org_pinned: Mutex::new(false),
        })
    }

    /// The adapter the saved settings describe: this provider, the API key
    /// transport, a team, and the key in Hollow's Keychain bundle.
    pub fn from_config(config: &TrackerConfig) -> Result<Self, TrackerError> {
        Self::configured(config, settings::saved_key(PROVIDER).map_err(|_| ()))
    }

    /// The same checks with a key the caller read.
    pub(crate) fn from_config_with_key(
        config: &TrackerConfig,
        key: &str,
    ) -> Result<Self, TrackerError> {
        Self::configured(config, Ok(Some(key.to_string())))
    }

    fn configured(
        config: &TrackerConfig,
        key: Result<Option<String>, ()>,
    ) -> Result<Self, TrackerError> {
        match config.provider.as_deref() {
            Some(PROVIDER) => {}
            Some(_) => {
                return Err(TrackerError::NotConfigured(
                    "another tracker is chosen".into(),
                ))
            }
            None => return Err(TrackerError::NotConfigured("no tracker is chosen".into())),
        }
        if config.transport != "graphql" {
            return Err(TrackerError::NotConfigured(
                "only the API key transport is built so far".into(),
            ));
        }
        let team = config
            .team_id
            .as_deref()
            .ok_or_else(|| TrackerError::NotConfigured("no team is set".into()))?;
        if team != TEAM_ID {
            return Err(TrackerError::NotConfigured(TEAM_ONLY.into()));
        }
        let personal = config.personal_team_id.as_deref() == Some(PERSONAL_TEAM_ID);
        match key {
            Ok(Some(key)) => Self::new(team, &key).map(|a| a.with_personal(personal)),
            Ok(None) => Err(TrackerError::NotConfigured("no API key is saved".into())),
            Err(()) => Err(TrackerError::NotConfigured(
                "the saved API key couldn't be read".into(),
            )),
        }
    }

    /// A create may name its team; this adapter is scoped to one, so any
    /// other is refused before a request (the engine picks the adapter that
    /// matches). None names the adapter's own.
    fn names_this_team(&self, team: Option<&str>) -> Result<(), TrackerError> {
        match team {
            None => Ok(()),
            Some(team) if team == self.team_id => Ok(()),
            Some(_) => Err(TrackerError::OutOfScope),
        }
    }

    pub(crate) fn with_personal(mut self, on: bool) -> Self {
        self.personal = on;
        self
    }

    /// Whether the config set up the personal team.
    pub fn personal_enabled(&self) -> bool {
        self.personal
    }

    #[cfg(test)]
    fn with_backoff(mut self, base: Duration, max: Duration) -> Self {
        self.backoff_base = base;
        self.backoff_max = max;
        self
    }

    /// The team every call is scoped to.
    #[allow(dead_code)] // A caller may inspect the configured scope without another network read.
    pub fn team_id(&self) -> &str {
        &self.team_id
    }

    /// The same adapter (client, key, rate state copied) scoped to another
    /// allowed team: the sync engine runs one pass per team, each through
    /// an adapter that names only that team.
    pub fn scoped_to(&self, team_id: &str) -> Result<Self, TrackerError> {
        let team_id = team_id.trim();
        if !team_allowed(team_id) {
            return Err(TrackerError::NotConfigured(TEAM_ONLY.into()));
        }
        Ok(LinearAdapter {
            http: self.http.clone(),
            url: self.url.clone(),
            auth: self.auth.clone(),
            team_id: team_id.to_string(),
            personal: self.personal,
            rate: Mutex::new(lock(&self.rate).clone()),
            points: Mutex::new(*lock(&self.points)),
            backoff: Mutex::new(lock(&self.backoff).clone()),
            backoff_base: self.backoff_base,
            backoff_max: self.backoff_max,
            org_pinned: Mutex::new(*lock(&self.org_pinned)),
        })
    }

    /// The last reply's complexity cost (Linear's `X-Complexity` header),
    /// when it sent one.
    #[cfg(test)]
    pub fn last_points(&self) -> Option<u64> {
        *lock(&self.points)
    }

    fn team_filter(&self) -> Value {
        json!({ "id": { "eq": self.team_id } })
    }

    /// Inside a backoff window: the error to return without a request.
    fn waiting(&self) -> Option<TrackerError> {
        let backoff = lock(&self.backoff);
        match backoff.until {
            Some(until) if until > Utc::now() => Some(TrackerError::RateLimited {
                reset_at: Some(stamp(until)),
            }),
            _ => None,
        }
    }

    /// A rate-limited reply: start or double the backoff window.
    fn rate_limited(&self) -> TrackerError {
        let mut backoff = lock(&self.backoff);
        backoff.streak = backoff.streak.saturating_add(1);
        let delay = backoff_delay(self.backoff_base, self.backoff_max, backoff.streak);
        let delay =
            chrono::Duration::from_std(delay).unwrap_or_else(|_| chrono::Duration::minutes(15));
        let until = Utc::now() + delay;
        backoff.until = Some(until);
        TrackerError::RateLimited {
            reset_at: Some(stamp(until)),
        }
    }

    fn note_headers(&self, headers: &HeaderMap) {
        if let Some(budget) = rate_from(headers) {
            *lock(&self.rate) = Some(budget);
        }
        if let Some(points) = header_number(headers, "x-complexity") {
            *lock(&self.points) = Some(points);
        }
    }

    /// One GraphQL request; its `data` on success. Fails at once inside a
    /// backoff window.
    async fn post(&self, query: &str, variables: Value) -> Result<Value, TrackerError> {
        if let Some(waiting) = self.waiting() {
            return Err(waiting);
        }
        if query.trim_start().starts_with("mutation") {
            self.confirm_org().await?;
        }
        let response = self
            .http
            .post(&self.url)
            .header(AUTHORIZATION, self.auth.clone())
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(|error| self.scrubbed(network(error)))?;
        let status = response.status().as_u16();
        self.note_headers(response.headers());
        let text = response
            .text()
            .await
            .map_err(|error| self.scrubbed(network(error)))?;
        let body = serde_json::from_str::<Value>(&text).ok();
        match classify(status, body.as_ref(), query) {
            Some(TrackerError::RateLimited { .. }) => Err(self.rate_limited()),
            Some(error) => Err(self.scrubbed(error)),
            None => {
                *lock(&self.backoff) = Backoff::default();
                body.and_then(|mut body| body.get_mut("data").map(Value::take))
                    .filter(|data| !data.is_null())
                    .ok_or_else(|| TrackerError::Provider("the reply had no data".into()))
            }
        }
    }

    /// Ok when the key sees a team beyond AI and HOL, so it isn't limited to
    /// them; an error otherwise (or when the list is cut short or odd).
    async fn sees_other_teams(&self) -> Result<(), TrackerError> {
        let data = self.post(TEAMS_VISIBLE_QUERY, json!({})).await?;
        let nodes = at(data, &["teams"])?
            .get("nodes")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| malformed("visible teams"))?;
        if nodes.iter().any(|t| {
            t.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !team_allowed(id))
        }) {
            return Ok(());
        }
        Err(TrackerError::Provider(
            "the key sees no team besides AI and Personal, so a create moved to a team it can't see would read as missing; the attempted journal is kept".into(),
        ))
    }

    /// A write goes only to Example Org's organization (Q-R25): the
    /// key's organization id is read once and must be [`ORG_ID`], as
    /// `tracker.graphql` asks before every call. Nothing is written when it
    /// is another's, or can't be read.
    async fn confirm_org(&self) -> Result<(), TrackerError> {
        if *lock(&self.org_pinned) {
            return Ok(());
        }
        let reply = self.raw(super::graphql::ORG_QUERY, json!({})).await?;
        let id = reply
            .pointer("/data/viewer/organization/id")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        if id != super::graphql::ORG_ID {
            return Err(TrackerError::NotConfigured(format!(
                "the saved API key belongs to another organization than {:?}; nothing was written",
                super::graphql::ORG_NAME
            )));
        }
        *lock(&self.org_pinned) = true;
        Ok(())
    }

    /// The GraphQL endpoint this adapter posts to (the rate-limit window's key).
    pub(crate) fn endpoint(&self) -> &str {
        &self.url
    }

    /// One GraphQL request for `tracker.graphql`: the reply's whole JSON body
    /// (`data` and `errors`, so the agent reads the server's own words) and
    /// Linear's rate-limit headers as `rateLimit`. A rate limit, an
    /// authentication failure or a reply that is not JSON is an error, as in
    /// [`Self::post`]; the key never appears in either.
    pub(crate) async fn raw(&self, query: &str, variables: Value) -> Result<Value, TrackerError> {
        if let Some(waiting) = self.waiting() {
            return Err(waiting);
        }
        let response = self
            .http
            .post(&self.url)
            .header(AUTHORIZATION, self.auth.clone())
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(|error| self.scrubbed(network(error)))?;
        let status = response.status().as_u16();
        self.note_headers(response.headers());
        let text = response
            .text()
            .await
            .map_err(|error| self.scrubbed(network(error)))?;
        let body = serde_json::from_str::<Value>(&text).ok();
        match classify(status, body.as_ref(), query) {
            Some(TrackerError::RateLimited { .. }) => Err(self.rate_limited()),
            Some(TrackerError::Auth) => Err(TrackerError::Auth),
            Some(error) if body.is_none() => Err(self.scrubbed(error)),
            _ => {
                *lock(&self.backoff) = Backoff::default();
                let mut body = body.filter(Value::is_object).ok_or_else(|| {
                    TrackerError::Provider("the reply was not a JSON object".into())
                })?;
                if let Some(rate) = lock(&self.rate).clone() {
                    body["rateLimit"] = json!({
                        "requestsRemaining": rate.requests_remaining,
                        "requestsLimit": rate.requests_limit,
                        "complexityRemaining": rate.complexity_remaining,
                        "complexityLimit": rate.complexity_limit,
                        "resetAt": rate.reset_at,
                    });
                }
                Ok(body)
            }
        }
    }

    /// `text` with every occurrence of the API key removed.
    pub(crate) fn without_key(&self, text: &str) -> String {
        let key = self.auth.to_str().unwrap_or("");
        if key.is_empty() {
            text.to_string()
        } else {
            text.replace(key, "••••")
        }
    }

    /// An error whose text names the key loses that text (`classify`
    /// already keeps none of the server's own words; this is the backstop).
    fn scrubbed(&self, error: TrackerError) -> TrackerError {
        let key = self.auth.to_str().unwrap_or("");
        let names_key = |text: &str| !key.is_empty() && text.contains(key);
        match error {
            TrackerError::Provider(text) if names_key(&text) => {
                TrackerError::Provider("an error that echoed the request".into())
            }
            TrackerError::Network(text) if names_key(&text) => {
                TrackerError::Network("a failure that echoed the request".into())
            }
            other => other,
        }
    }

    /// Every node of a connection at `path`, page after page.
    async fn all_nodes(
        &self,
        query: &str,
        mut variables: Value,
        path: &[&str],
    ) -> Result<Vec<Value>, TrackerError> {
        let mut out = Vec::new();
        let mut cursors = BTreeSet::new();
        for _ in 0..MAX_PAGES {
            let data = self.post(query, variables.clone()).await?;
            let (nodes, next) = connection(at(data, path)?);
            out.extend(nodes);
            let Some(cursor) = next else {
                return Ok(out);
            };
            if !cursors.insert(cursor.clone()) {
                return Err(TrackerError::Provider(
                    "the tracker repeated a page cursor".into(),
                ));
            }
            variables["after"] = json!(cursor);
        }
        Err(TrackerError::Provider(
            "more pages than one pass reads".into(),
        ))
    }

    fn in_scope(&self, nodes: Vec<Value>) -> Vec<Value> {
        keep_in_scope(
            &self.team_id,
            nodes,
            |node| team_of(node).unwrap_or(""),
            |node| str_ref(node, "id"),
        )
    }

    /// Whether `url` may take an upload: Linear's storage over HTTPS, or the
    /// adapter's own origin (the tests' stub serves both).
    fn upload_target(&self, url: &str) -> bool {
        let Ok(parsed) = reqwest::Url::parse(url) else {
            return false;
        };
        let own = reqwest::Url::parse(&self.url).ok();
        parsed.scheme() == "https"
            || own.is_some_and(|own| own.origin() == parsed.origin() && parsed.has_host())
    }

    /// An issues filter for one issue in the team, by id or by key ("ENG-37").
    fn issue_filter(&self, id_or_key: &str) -> Value {
        let mut filter = json!({ "team": self.team_filter() });
        match issue_number(id_or_key) {
            Some(number) => filter["number"] = json!({ "eq": number }),
            None => filter["id"] = json!({ "eq": id_or_key.trim() }),
        }
        filter
    }

    /// The id of a mutation's target issue when it is in the team. Another
    /// team's issue and a missing one look the same from inside the team,
    /// and Hollow never reads outside it to tell them apart: both refuse.
    async fn target_issue(&self, id_or_key: &str) -> Result<String, TrackerError> {
        let data = self
            .post(
                ISSUE_TARGET_QUERY,
                json!({ "filter": self.issue_filter(id_or_key) }),
            )
            .await?;
        let (nodes, _) = connection(at(data, &["issues"])?);
        let node = nodes
            .into_iter()
            .find(|node| names(node, id_or_key))
            .ok_or(TrackerError::OutOfScope)?;
        if team_of(&node) != Some(self.team_id.as_str()) {
            dropped(str_ref(&node, "id"), "issue");
            return Err(TrackerError::OutOfScope);
        }
        Ok(str_of(&node, "id"))
    }

    /// The id of a mutation's target project when it belongs to the team
    /// alone. A project shared with another team is read, never written.
    async fn target_project(&self, id: &str) -> Result<String, TrackerError> {
        let filter =
            json!({ "id": { "eq": id.trim() }, "accessibleTeams": { "some": self.team_filter() } });
        let data = self
            .post(PROJECT_TARGET_QUERY, json!({ "filter": filter }))
            .await?;
        let (nodes, _) = connection(at(data, &["projects"])?);
        let node = nodes
            .into_iter()
            .find(|node| str_ref(node, "id") == id.trim())
            .ok_or(TrackerError::OutOfScope)?;
        let teams = team_ids(&node);
        if teams.is_empty() || teams.iter().any(|team| team != &self.team_id) {
            return Err(TrackerError::OutOfScope);
        }
        Ok(str_of(&node, "id"))
    }

    /// One issue with every detail field, comments and attachments included.
    async fn detail(&self, id_or_key: &str) -> Result<TrackerIssue, TrackerError> {
        let data = self
            .post(
                &detail_query(),
                json!({ "filter": self.issue_filter(id_or_key) }),
            )
            .await?;
        let (nodes, _) = connection(at(data, &["issues"])?);
        let nodes = self.in_scope(nodes);
        let node = nodes
            .iter()
            .find(|node| names(node, id_or_key))
            .ok_or(TrackerError::NotFound)?;
        self.issue_from(node).ok_or_else(|| malformed("issue"))
    }

    fn issue_reply(&self, data: Value, field: &str) -> Result<TrackerIssue, TrackerError> {
        let node = at(applied(at(data, &[field])?)?, &["issue"])?;
        if team_of(&node) != Some(self.team_id.as_str()) {
            dropped(str_ref(&node, "id"), "issue");
            return Err(TrackerError::OutOfScope);
        }
        self.issue_from(&node).ok_or_else(|| malformed("issue"))
    }

    fn project_reply(&self, data: Value, field: &str) -> Result<TrackerProject, TrackerError> {
        let node = at(applied(at(data, &[field])?)?, &["project"])?;
        if !team_ids(&node).contains(&self.team_id) {
            dropped(str_ref(&node, "id"), "project");
            return Err(TrackerError::OutOfScope);
        }
        project_from(&node, &self.team_id).ok_or_else(|| malformed("project"))
    }

    /// A Linear issue node as a [`TrackerIssue`]; nested nodes from another
    /// team (a parent, a sub-issue, a related issue) are left out.
    fn issue_from(&self, node: &Value) -> Option<TrackerIssue> {
        let team = self.team_id.as_str();
        let id = str_ref(node, "id");
        if id.is_empty() {
            return None;
        }
        let in_team = |other: &Value| team_of(other) == Some(team);
        let parent_id = match node.get("parent").filter(|parent| !parent.is_null()) {
            Some(parent) if in_team(parent) => opt_str(parent, "id"),
            Some(_) => {
                dropped(id, "parent");
                None
            }
            None => None,
        };
        let children = nodes_of(node, "children")
            .iter()
            .filter_map(|child| {
                if in_team(child) {
                    opt_str(child, "identifier")
                } else {
                    dropped(id, "sub-issue");
                    None
                }
            })
            .collect();
        let mut relations: Vec<TrackerRelation> = Vec::new();
        for (list, other_key, inverse) in [
            ("relations", "relatedIssue", false),
            ("inverseRelations", "issue", true),
        ] {
            for relation in nodes_of(node, list) {
                let Some(other) = relation.get(other_key).filter(|other| !other.is_null()) else {
                    continue;
                };
                if !in_team(other) {
                    dropped(id, "relation");
                    continue;
                }
                let kind = match (str_ref(relation, "type"), inverse) {
                    ("blocks", false) => RelationKind::Blocks,
                    ("blocks", true) => RelationKind::BlockedBy,
                    ("duplicate", false) => RelationKind::DuplicateOf,
                    ("related", _) => RelationKind::Related,
                    _ => continue,
                };
                let Some(key) = opt_str(other, "identifier") else {
                    continue;
                };
                let relation = TrackerRelation { kind, key };
                if !relations.contains(&relation) {
                    relations.push(relation);
                }
            }
        }
        let (links, attachments) = split_links(
            nodes_of(node, "attachments")
                .iter()
                .filter_map(link_from)
                .collect(),
        );
        Some(TrackerIssue {
            id: id.to_string(),
            key: str_of(node, "identifier"),
            url: str_of(node, "url"),
            title: str_of(node, "title"),
            description: str_of(node, "description"),
            state: state_from(node.get("state")?)?,
            assignee: node.get("assignee").and_then(user_from),
            priority: node
                .get("priority")
                .and_then(Value::as_f64)
                .map_or(0, |priority| priority.clamp(0.0, 4.0) as u8),
            due: opt_str(node, "dueDate"),
            project_id: node
                .get("project")
                .and_then(|project| opt_str(project, "id")),
            labels: nodes_of(node, "labels")
                .iter()
                .filter_map(|label| opt_str(label, "name"))
                .collect(),
            parent_id,
            team_id: team.to_string(),
            updated_at: str_of(node, "updatedAt"),
            archived_at: opt_str(node, "archivedAt"),
            trashed: node
                .get("trashed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            estimate: node.get("estimate").and_then(Value::as_f64),
            cycle: node.get("cycle").and_then(cycle_from),
            milestone: node.get("projectMilestone").and_then(milestone_from),
            creator: node.get("creator").and_then(user_from),
            created_at: str_of(node, "createdAt"),
            completed_at: opt_str(node, "completedAt"),
            children,
            relations,
            links,
            attachments,
            comments: nodes_of(node, "comments")
                .iter()
                .filter_map(comment_from)
                .collect(),
        })
    }
}

impl TrackerAdapter for LinearAdapter {
    fn provider(&self) -> &'static str {
        PROVIDER
    }

    fn display_name(&self) -> &str {
        DISPLAY_NAME
    }

    fn for_team(&self, team: &str) -> Option<Result<Box<dyn TrackerAdapter>, TrackerError>> {
        if team == self.team_id {
            return None;
        }
        Some(
            self.scoped_to(team)
                .map(|adapter| Box::new(adapter) as Box<dyn TrackerAdapter>),
        )
    }

    fn writable_fields(&self) -> &[Field] {
        WRITABLE_FIELDS
    }

    fn rate(&self) -> Option<RateBudget> {
        lock(&self.rate).clone()
    }

    fn whoami(&self) -> TrackerFuture<'_, TrackerUser> {
        Box::pin(async move {
            let viewer = at(self.post(VIEWER_QUERY, json!({})).await?, &["viewer"])?;
            user_from(&viewer).ok_or_else(|| malformed("viewer"))
        })
    }

    fn scope(&self) -> TrackerFuture<'_, TrackerTeam> {
        Box::pin(async move {
            let data = self
                .post(TEAM_QUERY, json!({ "team": self.team_id }))
                .await?;
            let team = at(data, &["team"])?;
            if str_ref(&team, "id") != self.team_id {
                return Err(TrackerError::OutOfScope);
            }
            Ok(TrackerTeam {
                id: str_of(&team, "id"),
                key: str_of(&team, "key"),
                name: str_of(&team, "name"),
            })
        })
    }

    fn states(&self) -> TrackerFuture<'_, Vec<TrackerState>> {
        Box::pin(async move {
            let nodes = self
                .all_nodes(
                    &states_query(),
                    json!({ "team": self.team_id }),
                    &["team", "states"],
                )
                .await?;
            Ok(self.in_scope(nodes).iter().filter_map(state_from).collect())
        })
    }

    fn labels(&self) -> TrackerFuture<'_, Vec<TrackerLabel>> {
        Box::pin(async move {
            // The team's own labels and the workspace's (which belong to no team).
            let filter =
                json!({ "or": [{ "team": self.team_filter() }, { "team": { "null": true } }] });
            let nodes = self
                .all_nodes(
                    &labels_query(),
                    json!({ "filter": filter }),
                    &["issueLabels"],
                )
                .await?;
            Ok(nodes
                .iter()
                .filter(|node| {
                    let team = team_of(node);
                    let keep = team.is_none() || team == Some(self.team_id.as_str());
                    if !keep {
                        dropped(str_ref(node, "id"), "label");
                    }
                    keep
                })
                .filter(|node| node.get("isGroup").and_then(Value::as_bool) != Some(true))
                .filter_map(label_from)
                .collect())
        })
    }

    fn members(&self) -> TrackerFuture<'_, Vec<TrackerUser>> {
        Box::pin(async move {
            let nodes = self
                .all_nodes(
                    &members_query(),
                    json!({ "team": self.team_id }),
                    &["team", "members"],
                )
                .await?;
            Ok(nodes
                .iter()
                .filter(|node| node.get("active").and_then(Value::as_bool) != Some(false))
                .filter_map(user_from)
                .collect())
        })
    }

    fn cycles(&self) -> TrackerFuture<'_, Vec<TrackerCycle>> {
        Box::pin(async move {
            let since = Utc::now() - chrono::Duration::days(CYCLE_DAYS_BACK);
            let variables = json!({
                "team": self.team_id,
                "filter": { "endsAt": { "gt": stamp(since) } },
            });
            let nodes = self
                .all_nodes(&cycles_query(), variables, &["team", "cycles"])
                .await?;
            Ok(nodes.iter().filter_map(cycle_from).collect())
        })
    }

    fn milestones<'a>(
        &'a self,
        project_ids: &'a [String],
    ) -> TrackerFuture<'a, Vec<TrackerMilestone>> {
        Box::pin(async move {
            let mut out = Vec::new();
            for chunk in project_ids.chunks(MILESTONE_BATCH) {
                let filter = json!({ "id": { "in": chunk }, "accessibleTeams": { "some": self.team_filter() } });
                let data = self
                    .post(&milestones_query(), json!({ "filter": filter }))
                    .await?;
                let (projects, _) = connection(at(data, &["projects"])?);
                for project in &projects {
                    let project_id = str_of(project, "id");
                    if !chunk.contains(&project_id) {
                        continue;
                    }
                    for node in nodes_of(project, "projectMilestones") {
                        if let Some(mut milestone) = milestone_from(node) {
                            milestone
                                .project_id
                                .get_or_insert_with(|| project_id.clone());
                            out.push(milestone);
                        }
                    }
                }
            }
            Ok(out)
        })
    }

    fn issues_changed<'a>(
        &'a self,
        since: Option<&'a str>,
        after: Option<&'a str>,
    ) -> TrackerFuture<'a, Page<TrackerIssue>> {
        Box::pin(async move {
            let mut filter = json!({ "team": self.team_filter() });
            if let Some(since) = since {
                filter["updatedAt"] = json!({ "gt": since });
            }
            let data = self
                .post(&issues_query(), json!({ "filter": filter, "after": after }))
                .await?;
            let (nodes, next) = connection(at(data, &["issues"])?);
            let mut items = Vec::new();
            for node in self.in_scope(nodes) {
                if truncated(&node) {
                    items.push(self.detail(str_ref(&node, "id")).await?);
                } else if let Some(issue) = self.issue_from(&node) {
                    items.push(issue);
                }
            }
            items.sort_by(|a, b| a.updated_at.cmp(&b.updated_at));
            Ok(Page { items, next })
        })
    }

    fn comments_changed<'a>(
        &'a self,
        since: Option<&'a str>,
        after: Option<&'a str>,
    ) -> TrackerFuture<'a, Page<TrackerCommentChange>> {
        Box::pin(async move {
            let mut filter = json!({ "issue": { "team": self.team_filter() } });
            if let Some(since) = since {
                filter["updatedAt"] = json!({ "gt": since });
            }
            let data = self
                .post(
                    &comments_query(),
                    json!({ "filter": filter, "after": after }),
                )
                .await?;
            let (nodes, next) = connection(at(data, &["comments"])?);
            let changes = nodes
                .iter()
                .filter_map(|node| {
                    // A comment on a project update or a document has no issue.
                    let issue = node.get("issue").filter(|issue| !issue.is_null());
                    Some(TrackerCommentChange {
                        issue_id: issue.map(|issue| str_of(issue, "id")).unwrap_or_default(),
                        team_id: issue.and_then(team_of).unwrap_or_default().to_string(),
                        updated_at: str_of(node, "updatedAt"),
                        comment: comment_from(node)?,
                    })
                })
                .collect();
            let mut items = keep_in_scope(
                &self.team_id,
                changes,
                |change| change.team_id.as_str(),
                |change| change.comment.id.as_str(),
            );
            items.sort_by(|a, b| a.updated_at.cmp(&b.updated_at));
            Ok(Page { items, next })
        })
    }

    fn attachments<'a>(
        &'a self,
        issue_ids: &'a [String],
    ) -> TrackerFuture<'a, Vec<(String, Vec<TrackerLink>)>> {
        Box::pin(async move {
            let mut out = Vec::new();
            for chunk in issue_ids.chunks(BATCH) {
                let filter = json!({ "team": self.team_filter(), "id": { "in": chunk } });
                let data = self
                    .post(&attachments_query(), json!({ "filter": filter }))
                    .await?;
                let (nodes, _) = connection(at(data, &["issues"])?);
                for node in self.in_scope(nodes) {
                    let links = nodes_of(&node, "attachments")
                        .iter()
                        .filter_map(link_from)
                        .collect();
                    out.push((str_of(&node, "id"), links));
                }
            }
            Ok(out)
        })
    }

    fn projects_changed<'a>(
        &'a self,
        since: Option<&'a str>,
    ) -> TrackerFuture<'a, Vec<TrackerProject>> {
        Box::pin(async move {
            let mut filter = json!({ "accessibleTeams": { "some": self.team_filter() } });
            if let Some(since) = since {
                filter["updatedAt"] = json!({ "gt": since });
            }
            let nodes = self
                .all_nodes(
                    &projects_query(),
                    json!({ "filter": filter }),
                    &["projects"],
                )
                .await?;
            Ok(nodes
                .iter()
                .filter(|node| {
                    let keep = team_ids(node).contains(&self.team_id);
                    if !keep {
                        dropped(str_ref(node, "id"), "project");
                    }
                    keep
                })
                .filter_map(|node| project_from(node, &self.team_id))
                .collect())
        })
    }

    fn issue<'a>(&'a self, id: &'a str) -> TrackerFuture<'a, TrackerIssue> {
        Box::pin(self.detail(id))
    }

    fn issue_comments<'a>(&'a self, id: &'a str) -> TrackerFuture<'a, Vec<TrackerCommentChange>> {
        Box::pin(async move {
            let target = self.target_issue(id).await?;
            let mut after: Option<String> = None;
            let mut cursors = BTreeSet::new();
            let mut ids = BTreeSet::new();
            let mut result = Vec::new();
            for _ in 0..MAX_PAGES {
                let data = self
                    .post(
                        &comments_query(),
                        json!({
                            "filter": {"issue": {"id": {"eq": target}, "team": self.team_filter()}},
                            "after": after
                        }),
                    )
                    .await?;
                let connection = at(data, &["comments"])?;
                let nodes = connection
                    .get("nodes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| malformed("complete comments"))?;
                let more = connection
                    .pointer("/pageInfo/hasNextPage")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| malformed("comment pagination"))?;
                for node in nodes {
                    let change = scoped_comment(node, &target, &self.team_id)?;
                    if !ids.insert(change.comment.id.clone()) {
                        return Err(malformed("duplicate comment in complete enumeration"));
                    }
                    result.push(change);
                }
                if !more {
                    return Ok(result);
                }
                let cursor = connection
                    .pointer("/pageInfo/endCursor")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| malformed("comment pagination"))?;
                if !cursors.insert(cursor.to_string()) {
                    return Err(malformed("repeated comment cursor"));
                }
                after = Some(cursor.to_string());
            }
            Err(malformed("incomplete comment enumeration"))
        })
    }

    fn issue_history<'a>(&'a self, id: &'a str) -> TrackerFuture<'a, TrackerHistory> {
        Box::pin(async move {
            let data = self
                .post(&history_query(), json!({ "filter": self.issue_filter(id) }))
                .await?;
            let (nodes, _) = connection(at(data, &["issues"])?);
            let nodes = self.in_scope(nodes);
            let node = nodes
                .iter()
                .find(|node| names(node, id))
                .ok_or(TrackerError::NotFound)?;
            let history = node.get("history");
            let entries = history
                .and_then(|history| history.get("nodes"))
                .and_then(Value::as_array)
                .ok_or_else(|| malformed("issue history"))?;
            let mut events: Vec<TrackerHistoryEvent> =
                entries.iter().flat_map(history_events).collect();
            events.sort_by(|a, b| a.at.cmp(&b.at));
            // Only the newest page is read; say when older entries exist.
            let more = history
                .and_then(|history| history.pointer("/pageInfo/hasNextPage"))
                .and_then(Value::as_bool)
                == Some(true);
            Ok(TrackerHistory { events, more })
        })
    }

    fn create_issue_comment<'a>(
        &'a self,
        id: &'a str,
        body: &'a str,
        authorize: &'a (dyn Fn() -> Result<(), TrackerError> + Send + Sync),
    ) -> TrackerFuture<'a, TrackerCommentChange> {
        Box::pin(async move {
            if body.trim().is_empty() || body.len() > 2_000 {
                return Err(malformed("activity body"));
            }
            let target = self.target_issue(id).await?;
            authorize()?;
            let data = self
                .post(
                    COMMENT_CREATE_MUTATION,
                    json!({"input": {"issueId": target, "body": body}}),
                )
                .await?;
            let payload = at(data, &["commentCreate"])?;
            if payload.get("success").and_then(Value::as_bool) != Some(true) {
                return Err(malformed("comment create receipt"));
            }
            let change = scoped_comment(&at(payload, &["comment"])?, &target, &self.team_id)?;
            if change.comment.body != body {
                return Err(malformed("comment create body"));
            }
            Ok(change)
        })
    }

    fn post_project_update<'a>(
        &'a self,
        project_id: &'a str,
        body: &'a str,
        health: ProjectHealth,
        authorize: &'a (dyn Fn() -> Result<(), TrackerError> + Send + Sync),
    ) -> TrackerFuture<'a, TrackerProjectUpdate> {
        Box::pin(async move {
            if body.trim().is_empty() {
                return Err(malformed("status body"));
            }
            let target = self.target_project(project_id).await?;
            authorize()?;
            let input = json!({ "projectId": target, "body": body, "health": health.as_str() });
            let data = self
                .post(STATUS_UPDATE_MUTATION, json!({ "input": input }))
                .await?;
            let payload = applied(at(data, &["projectUpdateCreate"])?)?;
            let update = at(payload, &["projectUpdate"])?;
            if update.pointer("/project/id").and_then(Value::as_str) != Some(target.as_str()) {
                return Err(malformed("status update project"));
            }
            Ok(TrackerProjectUpdate {
                id: opt_str(&update, "id").ok_or_else(|| malformed("status update"))?,
                url: opt_str(&update, "url"),
                created_at: opt_str(&update, "createdAt"),
            })
        })
    }

    fn create_label<'a>(&'a self, name: &'a str) -> TrackerFuture<'a, TrackerLabel> {
        Box::pin(async move {
            if name.trim().is_empty() {
                return Err(malformed("label name"));
            }
            // Only AI or HOL, whatever the adapter was built with.
            if !team_allowed(&self.team_id) {
                return Err(TrackerError::OutOfScope);
            }
            let input = json!({ "teamId": self.team_id, "name": name });
            let data = self
                .post(LABEL_CREATE_MUTATION, json!({ "input": input }))
                .await?;
            let payload = applied(at(data, &["issueLabelCreate"])?)?;
            let label = at(payload, &["issueLabel"])?;
            if team_of(&label) != Some(self.team_id.as_str()) {
                return Err(malformed("label create team"));
            }
            label_from(&label).ok_or_else(|| malformed("label create"))
        })
    }

    fn upload_file<'a>(
        &'a self,
        filename: &'a str,
        content_type: &'a str,
        bytes: &'a [u8],
    ) -> TrackerFuture<'a, TrackerUpload> {
        Box::pin(async move {
            if filename.trim().is_empty() || bytes.is_empty() || bytes.len() > UPLOAD_MAX_BYTES {
                return Err(TrackerError::Provider(
                    "a file to upload needs a name and at most 10 MB".into(),
                ));
            }
            let data = self
                .post(
                    FILE_UPLOAD_MUTATION,
                    json!({ "contentType": content_type, "filename": filename, "size": bytes.len() }),
                )
                .await?;
            let payload = applied(at(data, &["fileUpload"])?)?;
            let file = at(payload, &["uploadFile"])?;
            let upload_url = str_of(&file, "uploadUrl");
            let asset_url = str_of(&file, "assetUrl");
            if !self.upload_target(&upload_url) || !self.upload_target(&asset_url) {
                return Err(malformed("upload address"));
            }
            let mut request = self
                .http
                .put(&upload_url)
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .header(reqwest::header::CACHE_CONTROL, "public, max-age=31536000");
            let signed = file.get("headers").and_then(Value::as_array);
            for header in signed.into_iter().flatten() {
                let (Some(name), Some(value)) = (opt_str(header, "key"), opt_str(header, "value"))
                else {
                    continue;
                };
                // The signed address's own headers; never the API key.
                if name.eq_ignore_ascii_case("authorization") {
                    continue;
                }
                request = request.header(name, value);
            }
            let response = request
                .body(bytes.to_vec())
                .send()
                .await
                .map_err(|error| self.scrubbed(network(error)))?;
            if !response.status().is_success() {
                return Err(TrackerError::Provider(format!(
                    "the file storage answered HTTP {}",
                    response.status().as_u16()
                )));
            }
            Ok(TrackerUpload {
                asset_url,
                filename: filename.to_string(),
                size: bytes.len() as u64,
            })
        })
    }

    fn attach_upload<'a>(
        &'a self,
        issue_id: &'a str,
        upload: &'a TrackerUpload,
        title: &'a str,
        authorize: &'a (dyn Fn() -> Result<(), TrackerError> + Send + Sync),
    ) -> TrackerFuture<'a, TrackerLink> {
        Box::pin(async move {
            if !self.upload_target(&upload.asset_url) {
                return Err(TrackerError::Provider(
                    "an attachment needs a web address".into(),
                ));
            }
            let target = self.target_issue(issue_id).await?;
            authorize()?;
            let input = json!({
                "issueId": target,
                "url": upload.asset_url,
                "title": title,
                "subtitle": upload.filename,
            });
            let data = self
                .post(ATTACHMENT_CREATE_MUTATION, json!({ "input": input }))
                .await?;
            let payload = applied(at(data, &["attachmentCreate"])?)?;
            let attachment = at(payload, &["attachment"])?;
            if attachment.pointer("/issue/id").and_then(Value::as_str) != Some(target.as_str())
                || team_of(&attachment["issue"]) != Some(self.team_id.as_str())
            {
                return Err(malformed("attachment issue"));
            }
            link_from(&attachment).ok_or_else(|| malformed("attachment"))
        })
    }

    fn create_issue(&self, issue: NewIssue) -> TrackerFuture<'_, TrackerIssue> {
        Box::pin(async move {
            let mut issue = issue;
            if issue.title.trim().is_empty() {
                return Err(TrackerError::Provider("a new issue needs a title".into()));
            }
            if let Some(parent) = issue.parent_id.clone() {
                issue.parent_id = Some(self.target_issue(&parent).await?);
            }
            self.names_this_team(issue.team_id.as_deref())?;
            let input = new_issue_input(&issue, &self.team_id);
            let data = self
                .post(&issue_create_mutation(), json!({ "input": input }))
                .await?;
            self.issue_reply(data, "issueCreate")
        })
    }

    fn create_landing<'a>(&'a self, id: &'a str) -> TrackerFuture<'a, CreateLanding> {
        Box::pin(async move {
            let data = self.post(ISSUE_LANDING_QUERY, json!({ "id": id })).await?;
            let nodes = at(data, &["issues"])?
                .get("nodes")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| malformed("issue read-back"))?;
            match nodes.as_slice() {
                // Absent only when the key plainly sees every team: a create
                // moved into a team a restricted key can't see reads as empty
                // too, so then the journal stays `attempted` (R3, #1402).
                [] => {
                    self.sees_other_teams().await?;
                    Ok(CreateLanding::Absent)
                }
                [node] if str_ref(node, "id") == id => match team_of(node) {
                    Some(team) if team == self.team_id => {
                        self.detail(id).await.map(CreateLanding::InScope)
                    }
                    Some(_) => {
                        dropped(id, "issue");
                        Ok(CreateLanding::OtherTeam)
                    }
                    None => Err(malformed("issue read-back team")),
                },
                _ => Err(malformed("issue read-back")),
            }
        })
    }

    fn update_issue<'a>(
        &'a self,
        id: &'a str,
        patch: IssuePatch,
    ) -> TrackerFuture<'a, TrackerIssue> {
        Box::pin(async move {
            if patch.is_empty() {
                return self.detail(id).await;
            }
            let target = self.target_issue(id).await?;
            let mut patch = patch;
            if let Some(parent) = patch.parent_id.clone().flatten() {
                patch.parent_id = Some(Some(self.target_issue(&parent).await?));
            }
            let input = issue_patch_input(&patch);
            let data = self
                .post(
                    &issue_update_mutation(),
                    json!({ "id": target, "input": input }),
                )
                .await?;
            self.issue_reply(data, "issueUpdate")
        })
    }

    fn archive_issue<'a>(&'a self, id: &'a str) -> TrackerFuture<'a, ()> {
        Box::pin(async move {
            let target = self.target_issue(id).await?;
            let data = self
                .post(ISSUE_ARCHIVE_MUTATION, json!({ "id": target }))
                .await?;
            applied(at(data, &["issueArchive"])?).map(|_| ())
        })
    }

    fn create_project(&self, project: NewProject) -> TrackerFuture<'_, TrackerProject> {
        Box::pin(async move {
            if project.name.trim().is_empty() {
                return Err(TrackerError::Provider("a new project needs a name".into()));
            }
            self.names_this_team(project.team_id.as_deref())?;
            let input = new_project_input(&project, &self.team_id);
            let data = self
                .post(&project_create_mutation(), json!({ "input": input }))
                .await?;
            self.project_reply(data, "projectCreate")
        })
    }

    fn update_project<'a>(
        &'a self,
        id: &'a str,
        patch: ProjectPatch,
    ) -> TrackerFuture<'a, TrackerProject> {
        Box::pin(async move {
            let target = self.target_project(id).await?;
            let input = project_patch_input(&patch);
            let data = self
                .post(
                    &project_update_mutation(),
                    json!({ "id": target, "input": input }),
                )
                .await?;
            self.project_reply(data, "projectUpdate")
        })
    }

    fn add_link<'a>(
        &'a self,
        target: LinkTarget,
        url: &'a str,
        title: &'a str,
    ) -> TrackerFuture<'a, TrackerLink> {
        Box::pin(async move {
            let url = url.trim();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(TrackerError::Provider("a link needs a web address".into()));
            }
            match target {
                LinkTarget::Issue(id) => {
                    let issue = self.target_issue(&id).await?;
                    let data = self
                        .post(
                            &issue_link_mutation(),
                            json!({ "issueId": issue, "url": url, "title": title }),
                        )
                        .await?;
                    let payload = applied(at(data, &["attachmentLinkURL"])?)?;
                    link_from(&at(payload, &["attachment"])?).ok_or_else(|| malformed("link"))
                }
                LinkTarget::Project(id) => {
                    let project = self.target_project(&id).await?;
                    let input = json!({ "projectId": project, "url": url, "label": title });
                    let data = self
                        .post(PROJECT_LINK_MUTATION, json!({ "input": input }))
                        .await?;
                    let payload = applied(at(data, &["entityExternalLinkCreate"])?)?;
                    project_link_from(&at(payload, &["entityExternalLink"])?)
                        .ok_or_else(|| malformed("link"))
                }
            }
        })
    }
}

// ---- Inputs ----------------------------------------------------------------------

/// `IssueCreateInput`: always in the scoped team.
fn new_issue_input(issue: &NewIssue, team_id: &str) -> Value {
    let mut input = Map::new();
    input.insert("teamId".into(), json!(team_id));
    input.insert("title".into(), json!(issue.title.trim()));
    let optional = [
        ("description", issue.description.clone().map(Value::from)),
        ("stateId", issue.state_id.clone().map(Value::from)),
        ("priority", issue.priority.map(Value::from)),
        ("assigneeId", issue.assignee_id.clone().map(Value::from)),
        ("projectId", issue.project_id.clone().map(Value::from)),
        ("dueDate", issue.due.clone().map(Value::from)),
        ("parentId", issue.parent_id.clone().map(Value::from)),
        ("estimate", issue.estimate.map(Value::from)),
        ("cycleId", issue.cycle_id.clone().map(Value::from)),
        (
            "projectMilestoneId",
            issue.milestone_id.clone().map(Value::from),
        ),
    ];
    for (key, value) in optional {
        if let Some(value) = value {
            input.insert(key.into(), value);
        }
    }
    if !issue.label_ids.is_empty() {
        input.insert("labelIds".into(), json!(issue.label_ids));
    }
    // Hollow's own id (Q-R22): Linear refuses a second create with it.
    if let Some(id) = &issue.id {
        input.insert("id".into(), json!(id));
    }
    Value::Object(input)
}

/// `IssueUpdateInput`: only the touched fields; `Some(None)` sends null to
/// clear one. Never a team (an issue doesn't move teams from Hollow).
fn issue_patch_input(patch: &IssuePatch) -> Value {
    let fields = [
        ("title", patch.title.as_ref().map(|v| json!(v))),
        ("description", patch.description.as_ref().map(|v| json!(v))),
        ("stateId", patch.state_id.as_ref().map(|v| json!(v))),
        ("priority", patch.priority.map(|v| json!(v))),
        ("assigneeId", patch.assignee_id.as_ref().map(|v| json!(v))),
        ("labelIds", patch.label_ids.as_ref().map(|v| json!(v))),
        ("projectId", patch.project_id.as_ref().map(|v| json!(v))),
        ("dueDate", patch.due.as_ref().map(|v| json!(v))),
        ("parentId", patch.parent_id.as_ref().map(|v| json!(v))),
        ("estimate", patch.estimate.map(|v| json!(v))),
        ("cycleId", patch.cycle_id.as_ref().map(|v| json!(v))),
        (
            "projectMilestoneId",
            patch.milestone_id.as_ref().map(|v| json!(v)),
        ),
    ];
    Value::Object(
        fields
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key.to_string(), value)))
            .collect(),
    )
}

/// `ProjectCreateInput`: always in the scoped team, and only it.
fn new_project_input(project: &NewProject, team_id: &str) -> Value {
    let mut input = Map::new();
    input.insert("teamIds".into(), json!([team_id]));
    input.insert("name".into(), json!(project.name.trim()));
    let optional = [
        ("description", project.description.clone()),
        ("leadId", project.lead_id.clone()),
        ("startDate", project.start_date.clone()),
        ("targetDate", project.target_date.clone()),
        ("statusId", project.status_id.clone()),
    ];
    for (key, value) in optional {
        if let Some(value) = value {
            input.insert(key.into(), json!(value));
        }
    }
    Value::Object(input)
}

/// `ProjectUpdateInput`: only the touched fields; never its teams.
fn project_patch_input(patch: &ProjectPatch) -> Value {
    let fields = [
        ("name", patch.name.as_ref().map(|v| json!(v))),
        ("description", patch.description.as_ref().map(|v| json!(v))),
        ("leadId", patch.lead_id.as_ref().map(|v| json!(v))),
        ("startDate", patch.start_date.as_ref().map(|v| json!(v))),
        ("targetDate", patch.target_date.as_ref().map(|v| json!(v))),
        ("statusId", patch.status_id.as_ref().map(|v| json!(v))),
    ];
    Value::Object(
        fields
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key.to_string(), value)))
            .collect(),
    )
}

// ---- Replies ---------------------------------------------------------------------

/// Linear's error codes (`extensions.code`, or `extensions.type` written
/// the same way). Only these reach an error's text.
const KNOWN_CODES: &[&str] = &[
    "INVALID_INPUT",
    "BAD_USER_INPUT",
    "GRAPHQL_VALIDATION_FAILED",
    "GRAPHQL_ERROR",
    "INTERNAL_ERROR",
    "INTERNAL_SERVER_ERROR",
    "FEATURE_NOT_ACCESSIBLE",
    "USAGE_LIMIT_EXCEEDED",
    "CONFLICT",
    "LOCK_TIMEOUT",
    "USER_ERROR",
    "NETWORK_ERROR",
    "BOOTSTRAP_ERROR",
    "OTHER",
];

/// The error a reply carries, if any: GraphQL errors first (Linear sends
/// most with 200 or 400), then the HTTP status. Nothing the server wrote
/// reaches the error's text (a message can echo what was sent, an issue's
/// title or the key): a provider error names a known code and the path's
/// fields that appear in `query`, the text Hollow itself sent.
fn classify(status: u16, body: Option<&Value>, query: &str) -> Option<TrackerError> {
    let first = body
        .and_then(|body| body.get("errors"))
        .and_then(Value::as_array)
        .and_then(|errors| errors.first());
    if let Some(error) = first {
        let code = error
            .pointer("/extensions/code")
            .or_else(|| error.pointer("/extensions/type"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase()
            .replace([' ', '-'], "_");
        let message = error.get("message").and_then(Value::as_str).unwrap_or("");
        return Some(match code.as_str() {
            "RATELIMITED" | "RATE_LIMITED" => TrackerError::RateLimited { reset_at: None },
            "AUTHENTICATION_ERROR" | "UNAUTHENTICATED" => TrackerError::Auth,
            "FORBIDDEN" => TrackerError::OutOfScope,
            _ if status == 429 => TrackerError::RateLimited { reset_at: None },
            _ if status == 401 => TrackerError::Auth,
            "NOT_FOUND" | "ENTITY_NOT_FOUND" => TrackerError::NotFound,
            _ if message.to_ascii_lowercase().contains("not found") => TrackerError::NotFound,
            _ => TrackerError::Provider(described(&code, error.get("path"), query)),
        });
    }
    match status {
        200..=299 => None,
        429 => Some(TrackerError::RateLimited { reset_at: None }),
        401 => Some(TrackerError::Auth),
        other => Some(TrackerError::Provider(format!("HTTP {other}"))),
    }
}

/// `INVALID_INPUT at issueUpdate`: a known code, and the GraphQL path's
/// fields that are words of the query Hollow sent; nothing else the server
/// wrote.
fn described(code: &str, path: Option<&Value>, query: &str) -> String {
    let code = if KNOWN_CODES.contains(&code) {
        code
    } else {
        "an unrecognized error"
    };
    let sent: BTreeSet<&str> = query
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .collect();
    let fields: Vec<&str> = path
        .and_then(Value::as_array)
        .map(|path| {
            path.iter()
                .filter_map(Value::as_str)
                .filter(|field| sent.contains(field))
                .collect()
        })
        .unwrap_or_default();
    if fields.is_empty() {
        code.to_string()
    } else {
        format!("{code} at {}", fields.join("."))
    }
}

fn network(error: reqwest::Error) -> TrackerError {
    if error.is_timeout() {
        TrackerError::Network("the request timed out".into())
    } else {
        TrackerError::Network(error.without_url().to_string())
    }
}

fn header_number(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

/// Linear's rate-limit headers; their resets are epoch milliseconds.
fn rate_from(headers: &HeaderMap) -> Option<RateBudget> {
    let number = |name: &str| header_number(headers, name);
    let reset = number("x-ratelimit-requests-reset")
        .or_else(|| number("x-ratelimit-complexity-reset"))
        .and_then(|ms| i64::try_from(ms).ok())
        .and_then(DateTime::<Utc>::from_timestamp_millis);
    let budget = RateBudget {
        requests_remaining: number("x-ratelimit-requests-remaining"),
        requests_limit: number("x-ratelimit-requests-limit"),
        complexity_remaining: number("x-ratelimit-complexity-remaining"),
        complexity_limit: number("x-ratelimit-complexity-limit"),
        reset_at: reset.map(stamp),
    };
    (budget != RateBudget::default()).then_some(budget)
}

/// The value at `path`, or the error its absence means.
fn at(mut value: Value, path: &[&str]) -> Result<Value, TrackerError> {
    for key in path {
        value = match value.get_mut(*key).map(Value::take) {
            Some(next) if !next.is_null() => next,
            _ if *key == "team" => {
                return Err(TrackerError::NotConfigured(
                    "the saved API key can't see the configured team".into(),
                ))
            }
            _ => return Err(TrackerError::Provider(format!("the reply had no {key}"))),
        };
    }
    Ok(value)
}

/// A mutation payload, unless it says it wasn't applied.
fn applied(payload: Value) -> Result<Value, TrackerError> {
    if payload.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(TrackerError::Provider(
            "the tracker didn't apply the change".into(),
        ));
    }
    Ok(payload)
}

/// A connection's nodes and the cursor of its next page.
fn connection(mut conn: Value) -> (Vec<Value>, Option<String>) {
    let nodes = match conn.get_mut("nodes").map(Value::take) {
        Some(Value::Array(nodes)) => nodes,
        _ => Vec::new(),
    };
    let more = conn
        .pointer("/pageInfo/hasNextPage")
        .and_then(Value::as_bool)
        == Some(true);
    let next = if more {
        opt_str(&conn["pageInfo"], "endCursor")
    } else {
        None
    };
    (nodes, next)
}

/// A list node whose nested lists may have been cut at [`NESTED_PAGE`].
fn truncated(node: &Value) -> bool {
    ["labels", "children", "relations", "inverseRelations"]
        .iter()
        .any(|list| nodes_of(node, list).len() >= NESTED_PAGE as usize)
}

/// "ENG-37" as 37; None for an id.
fn issue_number(key: &str) -> Option<u64> {
    let (prefix, number) = key.trim().rsplit_once('-')?;
    let prefix_ok = prefix
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && prefix.chars().all(|c| c.is_ascii_alphanumeric());
    if !prefix_ok || number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

/// The node is the one asked for, by id or by key.
fn names(node: &Value, id_or_key: &str) -> bool {
    let wanted = id_or_key.trim();
    str_ref(node, "id") == wanted || str_ref(node, "identifier").eq_ignore_ascii_case(wanted)
}

fn str_ref<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn str_of(value: &Value, key: &str) -> String {
    str_ref(value, key).to_string()
}

fn opt_str(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn nodes_of<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(|conn| conn.get("nodes"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn team_of(value: &Value) -> Option<&str> {
    value
        .get("team")
        .and_then(|team| team.get("id"))
        .and_then(Value::as_str)
}

fn team_ids(value: &Value) -> Vec<String> {
    nodes_of(value, "teams")
        .iter()
        .filter_map(|team| opt_str(team, "id"))
        .collect()
}

fn malformed(what: &str) -> TrackerError {
    TrackerError::Provider(format!("the reply's {what} was incomplete"))
}

/// The scope guard's log line for a nested node (ids only, never text).
fn dropped(node_id: &str, what: &str) {
    eprintln!("[tracker] dropped out-of-scope {what} on {node_id}");
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn user_from(value: &Value) -> Option<TrackerUser> {
    Some(TrackerUser {
        id: opt_str(value, "id")?,
        name: str_of(value, "name"),
        email: opt_str(value, "email"),
        display_name: opt_str(value, "displayName"),
    })
}

fn state_from(value: &Value) -> Option<TrackerState> {
    Some(TrackerState {
        id: opt_str(value, "id")?,
        name: str_of(value, "name"),
        kind: StateKind::parse(str_ref(value, "type")).unwrap_or(StateKind::Backlog),
        position: value.get("position").and_then(Value::as_f64).unwrap_or(0.0),
        color: opt_str(value, "color"),
    })
}

fn label_from(value: &Value) -> Option<TrackerLabel> {
    Some(TrackerLabel {
        id: opt_str(value, "id")?,
        name: str_of(value, "name"),
        color: opt_str(value, "color"),
        group: value
            .get("parent")
            .and_then(|parent| opt_str(parent, "name")),
    })
}

fn cycle_from(value: &Value) -> Option<TrackerCycle> {
    Some(TrackerCycle {
        id: opt_str(value, "id")?,
        number: value
            .get("number")
            .and_then(Value::as_f64)
            .map_or(0, |number| number.max(0.0) as u32),
        name: opt_str(value, "name"),
        starts_at: str_of(value, "startsAt"),
        ends_at: str_of(value, "endsAt"),
    })
}

fn milestone_from(value: &Value) -> Option<TrackerMilestone> {
    Some(TrackerMilestone {
        id: opt_str(value, "id")?,
        name: str_of(value, "name"),
        target_date: opt_str(value, "targetDate"),
        project_id: value
            .get("project")
            .and_then(|project| opt_str(project, "id")),
    })
}

fn scoped_comment(
    value: &Value,
    issue_id: &str,
    team_id: &str,
) -> Result<TrackerCommentChange, TrackerError> {
    if value.pointer("/issue/id").and_then(Value::as_str) != Some(issue_id)
        || value.pointer("/issue/team/id").and_then(Value::as_str) != Some(team_id)
    {
        return Err(TrackerError::OutOfScope);
    }
    if ["id", "body", "createdAt", "updatedAt"]
        .iter()
        .any(|key| value.get(key).and_then(Value::as_str).is_none())
    {
        return Err(malformed("comment receipt"));
    }
    Ok(TrackerCommentChange {
        issue_id: issue_id.into(),
        team_id: team_id.into(),
        updated_at: str_of(value, "updatedAt"),
        comment: comment_from(value).ok_or_else(|| malformed("comment receipt"))?,
    })
}

fn comment_from(value: &Value) -> Option<TrackerComment> {
    Some(TrackerComment {
        id: opt_str(value, "id")?,
        author: value.get("user").and_then(|user| opt_str(user, "name")),
        body: str_of(value, "body"),
        created_at: str_of(value, "createdAt"),
        edited_at: opt_str(value, "editedAt"),
        parent_id: value.get("parent").and_then(|parent| opt_str(parent, "id")),
    })
}

/// One Linear history entry as an event per field it changed. An entry with
/// none of the fields Hollow shows (a cycle, an estimate, a link) gives none.
fn history_events(entry: &Value) -> Vec<TrackerHistoryEvent> {
    let Some(id) = opt_str(entry, "id") else {
        return Vec::new();
    };
    let at = str_of(entry, "createdAt");
    let actor = entry.get("actor").and_then(|actor| opt_str(actor, "name"));
    // A side of a change: absent or null is no value, an object with a string
    // `name` is that value, and anything else is a malformed field (`Err`).
    let side = |key: &str| -> Result<Option<String>, ()> {
        match entry.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(node) => match node.get("name").and_then(Value::as_str) {
                Some(name) => Ok(Some(name.to_string()).filter(|n| !n.is_empty())),
                None => Err(()),
            },
        }
    };
    // Both sides of a named field, or `None` when either is malformed or
    // neither carries a value (nothing to show).
    let pair = |from: &str, to: &str| -> Option<(Option<String>, Option<String>)> {
        let (f, t) = (side(from).ok()?, side(to).ok()?);
        (f.is_some() || t.is_some()).then_some((f, t))
    };
    let names_in = |key: &str| -> Vec<String> {
        entry
            .get(key)
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(|l| opt_str(l, "name")).collect())
            .unwrap_or_default()
    };
    let event = |kind: HistoryKind, from: Option<String>, to: Option<String>| TrackerHistoryEvent {
        id: format!("{id}:{}", kind_word(kind)),
        at: at.clone(),
        actor: actor.clone(),
        kind,
        from,
        to,
        from_kind: None,
        to_kind: None,
        added: Vec::new(),
        removed: Vec::new(),
    };
    let mut out = Vec::new();
    if let Some((from, to)) = pair("fromState", "toState") {
        let kind_of = |key: &str| {
            entry
                .get(key)
                .and_then(|state| state.get("type"))
                .and_then(Value::as_str)
                .and_then(StateKind::parse)
        };
        out.push(TrackerHistoryEvent {
            from_kind: kind_of("fromState"),
            to_kind: kind_of("toState"),
            ..event(HistoryKind::State, from, to)
        });
    }
    if let Some((from, to)) = pair("fromAssignee", "toAssignee") {
        out.push(event(HistoryKind::Assignee, from, to));
    }
    // Priority is a number on both sides (the from side may be absent).
    let number = |key: &str| match entry.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(v) => v.as_f64().map(|p| Some((p as u8).to_string())),
    };
    if let (Some(from), Some(Some(to))) = (number("fromPriority"), number("toPriority")) {
        if from.as_ref() != Some(&to) {
            out.push(event(HistoryKind::Priority, from, Some(to)));
        }
    }
    let (added, removed) = (names_in("addedLabels"), names_in("removedLabels"));
    if !added.is_empty() || !removed.is_empty() {
        out.push(TrackerHistoryEvent {
            added,
            removed,
            ..event(HistoryKind::Labels, None, None)
        });
    }
    if let Some((from, to)) = pair("fromProject", "toProject") {
        out.push(event(HistoryKind::Project, from, to));
    }
    if let (Some(from), Some(to)) = (opt_str(entry, "fromTitle"), opt_str(entry, "toTitle")) {
        out.push(event(HistoryKind::Title, Some(from), Some(to)));
    }
    if entry.get("updatedDescription").and_then(Value::as_bool) == Some(true) {
        out.push(event(HistoryKind::Description, None, None));
    }
    out
}

fn kind_word(kind: HistoryKind) -> &'static str {
    match kind {
        HistoryKind::State => "state",
        HistoryKind::Assignee => "assignee",
        HistoryKind::Priority => "priority",
        HistoryKind::Labels => "labels",
        HistoryKind::Project => "project",
        HistoryKind::Title => "title",
        HistoryKind::Description => "description",
    }
}

/// An issue attachment; `source` is Linear's `sourceType` (`url` when it
/// sends none).
fn link_from(value: &Value) -> Option<TrackerLink> {
    Some(TrackerLink {
        id: opt_str(value, "id")?,
        title: str_of(value, "title"),
        url: str_of(value, "url"),
        source: Some(opt_str(value, "sourceType").unwrap_or_else(|| "url".into())),
        created_at: opt_str(value, "createdAt"),
    })
}

fn project_link_from(value: &Value) -> Option<TrackerLink> {
    Some(TrackerLink {
        id: opt_str(value, "id")?,
        title: str_of(value, "label"),
        url: str_of(value, "url"),
        source: Some("url".into()),
        created_at: opt_str(value, "createdAt"),
    })
}

/// Plain URL links apart from integrations (a pull request, a file).
fn split_links(all: Vec<TrackerLink>) -> (Vec<TrackerLink>, Vec<TrackerLink>) {
    all.into_iter()
        .partition(|link| link.source.as_deref().is_none_or(|source| source == "url"))
}

/// A project node as a [`TrackerProject`]. Its `team_ids` keep the scoped
/// team only: another team's id never leaves the adapter (a shared project
/// is still refused for writes by `target_project`).
fn project_from(value: &Value, team_id: &str) -> Option<TrackerProject> {
    let status = value.get("status");
    Some(TrackerProject {
        id: opt_str(value, "id")?,
        key: opt_str(value, "identifier"),
        url: str_of(value, "url"),
        name: str_of(value, "name"),
        description: str_of(value, "description"),
        status: TrackerProjectStatus {
            name: status.map(|s| str_of(s, "name")).unwrap_or_default(),
            kind: status.map(|s| str_of(s, "type")).unwrap_or_default(),
        },
        lead: value.get("lead").and_then(user_from),
        start_date: opt_str(value, "startDate"),
        target_date: opt_str(value, "targetDate"),
        team_ids: team_ids(value)
            .into_iter()
            .filter(|team| team == team_id)
            .collect(),
        updated_at: str_of(value, "updatedAt"),
        progress: value.get("progress").and_then(Value::as_f64),
        links: nodes_of(value, "externalLinks")
            .iter()
            .filter_map(project_link_from)
            .collect(),
    })
}

#[cfg(test)]
#[path = "linear_tests.rs"]
mod tests;
